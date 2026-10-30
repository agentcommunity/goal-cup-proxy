# Lane J: tests and gate

Appendix to the 30 Oct audit. Repo `/home/user/goal-cup-proxy`, commit 4b226de2 (origin/main). Read-only. No build,
no test run and no gate run: every line count is `wc -l` or a small script over the files, and every time comes
from a commit message or a doc. Where nothing was recorded, this file says "not recorded". Verdicts are
recommendations, not changes.

## Lane summary

Goal Cup Proxy has about 158,600 lines of tests (more than its 102,831 lines of `crates/*/src`, which already
include 16,335 lines of `#[cfg(test)]`), about 2,400 Rust test attributes, 276 Worker tests and 577 Python test
methods. About 53 percent of the integration, Worker and script test lines guard one of the seven points, often
only in part; 43 percent guard parts that the owner's decisions or a fresh build would cut; 4 percent are tooling
and glue. Several tests pin the very caps the owner wants removed, and no test proves the hard cases of the new
goal (hundreds of subagents, a recorder that keeps dying, URL passwords, a full client to Worker to `fetch-run`
rebuild). The gate itself (`scripts/gate.sh`, 113 lines) is sound and barely needs to change: its fast tier
took 54 s on a quiet machine and 306 s on a loaded sandbox, the full tier time is not recorded, and there is one
open flake, most likely a port reuse race that shows up at about 12 sites. Recommended outcome: delete about
60,000 test lines with the code they guard, rewrite about 61,000 down to about 30,000, keep about 37,000, add six
new tests, fix the port race and the sub-second time bounds, and change the README promise list first.

## Top findings

1. **The promise list blocks the cut.** README.md:278 and AGENTS.md:101 say a test that guards a promise
   (README.md:276-288, P1 to P7) is never deleted. P2 (chain catches a stray edit) and P5 (control API
   authenticates) guard none of the seven points; P3, P4 and P7 only partly match; points 2, 3, 5 and 6 have no
   promise at all. Replace P1 to P7 with R1 to R7 (section A3) in the same PR that deletes tests.
2. **Tests pin the caps the owner wants gone**: recorder restart cap of five
   (`m2_proxy_integration.rs:2967-3025`, `MAX_RECORDER_RESTARTS = 5` at `crates/goalcup-cli/src/proxy/mod.rs:1019-1022`),
   128 open streams, connections and live producers (`crates/goalcup-protocol/src/capture.rs:57-66`), 32 native
   streams and 1,024 native files (`crates/goalcup-daemon/src/native_capture.rs:272-284`), 1,024 connection tasks
   then a 503 (`crates/goalcup-cli/src/proxy/mod.rs:109-121`) and the IPC sender's ten-failure stop
   (`crates/goalcup-cli/src/proxy/ingest.rs:100-104`). The restart cap and the 503 break point 3 today.
3. **No test proves the new goal's hard cases**: past 32, 128 or 256 (macOS default) open files; a recorder that
   dies more than five times; URL passwords (they are masked today only by the email rule,
   `crates/goalcup-daemon/src/redact.rs:280-287`, which the owner wants dropped); the real client to the real
   Worker to `fetch-run`, ordered by ID and time.
4. **Cut parts carry about 68,000 test lines** (integration plus `#[cfg(test)]`): research decoders (17,264 plus
   1,154 cfg plus 2,182 fixture lines), the separate daemon and control API (6,495), seal and signed status
   (6,225), the Rust stand-in of the upload server (5,423), termination and spawn gate (5,145), live-run tooling
   (4,178), byte-for-byte goldens (3,640), crash and budget campaigns (2,881) and more.
5. **A minimum set of about 27,000 existing lines plus six new tests (N1 to N6)** proves the seven points end to
   end (section A7.5).
6. **The gate is simple and stays.** `sh scripts/gate.sh fast` before each commit (about 3 to 5 minutes warm on 4
   vCPU), `full` before the Friday tag and the 2-hour run. Cut `ci-scope.py` and take the link check and the
   script unit tests out of the gate if their code goes.
7. **One open flake, likely a test bug**: `an_upstream_outage_is_one_window_closed_by_the_next_reach` binds port 0,
   drops the listener and expects refusals; another test process can take the port. The same bind, drop and
   reuse pattern appears at about 12 sites. Fix it once.
8. **The 2-hour run report** (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any of the 89
   remote branches yet, so nothing here relies on it. The full gate time, CI time, Worker test time and
   linux-check time are not recorded anywhere found.

## Test lines by group

Three groups: tests that protect one of the seven points (including the local end-to-end and chain-format groups
the upload needs), tests that protect a part a cut removes, and tests that protect neither. Sums come from the
tables in sections A4.1 and A4.2.

| Group | Integration, Worker and script tests | `#[cfg(test)]` in `crates/*/src` | All test lines | Share | Keep / rewrite / delete (integration only) |
|---|---|---|---|---|---|
| Protects a point (1 to 7, E2E local, chain format) | 74,972 | 9,537 | 84,509 | 53 percent | 26,853 / 48,119 / 0 |
| Protects a cut part | 61,578 | 6,798 (incl. 18 lines of device login, kept) | 68,376 | 43 percent | 691 / 7,433 / 53,454 |
| Neither (release tooling, glue, CLI surface, code rule) | 5,756 | 0 | 5,756 | 4 percent | 4,974 / 782 / 0 |
| **Total** | **142,306** | **16,335** | **158,641** | | **32,518 / 56,334 / 53,454** |

Shares are of all 158,641 lines; on the integration set alone they are 53, 43 and 4 percent too (74,972, 61,578
and 5,756 of 142,306). Fixtures (205 files, 6,939 lines) are not in this table.

By point, integration tests only: point 1 wire capture 20,041; point 2 subagents 2,664; point 3 never block
5,807; point 4 secrets 9,846; point 5 upload 9,869; point 6 tell the participant 2,021; point 7 harnesses 14,781;
E2E local 6,346; chain format 3,597. Point 2, the owner's main concern, has the fewest test lines of any point,
and all of them need a rewrite.

## Where the two parts of this lane disagree or overlap

| Topic | Test map (Part A) | Gate review (Part B) | Note |
|---|---|---|---|
| `capture_failure_campaign` (755 lines) | Delete with the T25 crash and budget campaigns (owner: disk full gets no special handling) | Keep, release only, as one of the only full-pipeline checks (commit cffa1a38) | Both stated. If the disk-full code goes, its quota rows go; the full-pipeline role passes to `m2_synthetic_vertical` and the new N5. |
| `upload_worker.rs` | "`#[ignore]` and needs Node; make it part of the gate", "not `#[ignore]`" | Already run by the full tier (gate.sh:90-93) | Checked: gate.sh:90-93 runs it with `--run-ignored ignored-only`. It is in the full gate but not the fast tier or a plain `cargo test`. Part A's advice means: keep it in the full tier and make N4 part of it. |
| `test_fetch_run.py` (695 lines) | Keep: the server rebuild of a run (point 5) | Lists fetch-run among scripts the scripts lane may cut (about 345 of 577 Python tests go) | Both stated. This lane's view: keep `fetch-run.sh` and its test while it is how the server rebuilds a run. |
| `check-links.py` tests (1,672) and `check-version.py` tests (128) | Keep, as release and repo tooling | Cut `check-links.py` from the gate (keep in CI if wanted); `check-version.py` cut or keep (small) | Not a real conflict: Part A says keep the tests, Part B says where they run. |
| Test counts | Lines: 158,641 in all; Worker 5,800 lines; scripts 10,393 lines in 20 files | Tests: about 2,400 Rust attributes, 276 Worker tests, 577 Python test methods in 20 files | Different measures of the same files, both kept. |
| Live runs | N6, not in the gate | "Keep, by hand", one per supported harness, in the Vercel Sandbox | Same recommendation. |
| Many open files | N1 under `ulimit -n 256` | New tests that open hundreds of files could be flaky like the port race | Both stated: run N1 in the `serial` nextest group. |

# Part A: the test map

Repo `/home/user/goal-cup-proxy`, commit 4b226de2. Read-only. No build and no test run: every count
below is `wc -l` or a small script over the files. Verdicts are recommendations, not changes.

## A1. Test map summary

- There are about **158,600 lines of tests**: 126,112 in `crates/*/tests` (Rust integration tests),
  16,335 in `#[cfg(test)]` modules inside `crates/*/src`, 5,800 in `server/worker/test` and 10,393 in
  `scripts/test_*.py`. Fixtures add 205 files and 6,939 lines.
- About half of the integration, Worker and script test lines (53 percent) guard one of the seven
  points, many only in part. Another 43 percent guard parts that the owner's decisions or a fresh
  build would cut: the research decoders (17,264 lines), the separate daemon
  and its control API (6,495), seal and signed status (6,225), the Rust stand-in of the upload
  server (5,423), process termination and the spawn gate (5,145), live-run tooling (4,178),
  byte-for-byte goldens (3,640), crash and budget campaigns (2,881) and more.
- **Points 2, 3, 5 and 6 have no promise in README.** README P2 (the chain catches a stray edit) and
  P5 (the control API authenticates) guard none of the seven points. P3, P4 and P7 only partly match.
- **Several tests assert the caps the owner wants gone**, so they must be rewritten, not kept: the
  recorder restart cap of five, the 128 open streams and connections, the 32 native streams and
  1,024 native files, the 1,024 connection tasks (refused with a 503) and the ten-failure stop of
  the IPC sender.
- **No test covers the hard cases of the new goal**: hundreds of subagents at once (past 32, 128 and
  the macOS default of 256 open files), a recorder that dies more than five times, URL passwords,
  or a full client to Worker to `fetch-run` rebuild ordered by ID and time.
- Recommended outcome: delete about 60,000 lines (tests of cut parts), rewrite about 61,000 down to
  roughly half, keep about 37,000. A minimum set of about 27,000 lines, plus six new tests and
  the live runs, proves the seven points end to end.

## A2. Method and totals

- Integration tests: every `.rs` file under `crates/*/tests` (fixtures left out), every file in
  `server/worker/test`, every `scripts/test_*.py`. Each file got one primary group from its header
  doc comment and its test names (`#[test]`, `it(`, `def test`). A file that serves two groups is
  under its main one, and the note says the other.
- `#[cfg(test)]` modules: a brace-counting script found 91 inline modules (13,293 lines) plus two
  test files under `src` (`crates/goalcup-cli/src/participant/tests.rs`, 702;
  `crates/goalcup-daemon/src/runtime/capture/tests.rs`, 2,340). Each was grouped by its source file.
- "Delete" means delete together with the code the cut removes. If another lane keeps that part,
  its tests stay. "Rewrite" means keep the behaviour, drop rows for cut parts or caps, and shrink.

| Where | Files | Lines (tests) |
|---|---|---|
| `crates/goalcup-cli/tests` | 72 | 59,973 |
| `crates/goalcup-daemon/tests` | 32 | 30,260 |
| `crates/goalcup-verifier/tests` | 20 | 20,724 |
| `crates/goalcup-protocol/tests` | 19 | 13,471 |
| `crates/goalcup-platform/tests` | 8 | 1,684 |
| `#[cfg(test)]` in `crates/*/src` | 93 modules | 16,335 (cli 8,688, daemon 4,086, protocol 1,370, verifier 1,335, platform 856) |
| `server/worker/test` | 22 | 5,800 |
| `scripts/test_*.py` | 20 | 10,393 |
| Fixtures (`crates/*/tests/fixtures`) | 205 | 6,939 (cli 3,824, verifier 2,182, protocol 720, daemon 213) |

Source for comparison: `crates/*/src` holds 102,831 lines, and that figure includes the 16,335
lines of `#[cfg(test)]`.

## A3. The README promises against the seven points

README.md:276-288 lists P1 to P7. README.md:278 and AGENTS.md:101 say a test that guards one is
never deleted. That rule now protects tests of parts the owner wants cut, so the list must change
first.

| Promise | What it guards | Seven points | Still matches? |
|---|---|---|---|
| P1 No key fragment leaks | daemon redaction suites, connector contract, `hermes_offline_key.rs` | 4 | Yes, but too narrow: point 4 is every secret, not only provider keys. Add the URL password rule; the email rule goes. |
| P2 The chain catches a stray edit | `chain_golden.rs`, `m2_capture_integrity.rs`, `v5_corpus.rs` | none | No. Tamper evidence serves the event rules, not a point. Keep only `chain_golden.rs` (102 lines) if the upload still uses the link hash. |
| P3 Capture policy refuses endpoints, proxies, credentials and bad args | `harness_schema.rs`, conformance, launch plan, dependency bans | partly 1 and 7 | Partly. "No base URL, traffic goes through the proxy" serves point 1. Most of the 114 schema refusal rows do not. |
| P4 Connectors load and pins match | pins and loader tests, `shipped_connectors.rs` | partly 7 | Partly. Loading serves point 7. Pins are tamper checks. |
| P5 The control API authenticates | `control_api.rs` | none | No. It is local hardening of the daemon split. It goes if the daemon is folded into the CLI. |
| P6 The recording proxy captures faithfully | TLS, HTTP and WebSocket relay; verifier decoders | 1 (and 3: bytes reach the harness unchanged) | Yes for the relay tests. No for the decoders: they interpret, they do not record. |
| P7 One end-to-end capture per harness, and termination | lifecycle suites, contract kit, capture lifecycle, `process_kill.rs` | 7 and 1 | Half. End to end per harness: yes. Termination: no point. Also the offline kit has only two synthetic cases (`crates/goalcup-cli/tests/fixtures/harness-kit/`), not one per harness. |

Missing promises: point 2 (every subagent), point 3 (never block, note a gap), point 5 (the server
rebuilds the run by ID and time) and point 6 (the participant is told). Proposed new list:

| New | Promise | Proved by |
|---|---|---|
| R1 | Every model call through the proxy is recorded, to any provider | relay suites, `model_host_from.rs`, E2E |
| R2 | Every subagent file is recorded, however many | `capture_native.rs` core, new many-subagents test, E2E |
| R3 | Nothing we do blocks or slows the harness; a failure is a gap | the never-block rows listed in section A7, new restart test |
| R4 | No secret is recorded | redaction suites, `hermes_offline_key.rs`, run secrets, new URL password test |
| R5 | The server can rebuild the whole run by ID and time | `upload_chunks.rs`, Worker `integration.test.ts`, `test_fetch_run.py`, E2E |
| R6 | The participant is told plainly what was recorded | `participant/tests.rs`, `native_silence.rs`, finish lines |
| R7 | Each shipped harness runs the way people run it | `connector_contract` (static and full), kit, `test_install.py`, live runs |

## A4. Lines per group

### A4.1 Integration tests, Worker tests and script tests (142,306 lines)

| Group | Lines | Keep | Rewrite | Delete |
|---|---|---|---|---|
| Point 1: wire capture (proxy relay, TLS, daemon receiver) | 20,041 | 7,720 | 12,321 | 0 |
| Point 2: subagents (native files) | 2,664 | 0 | 2,664 | 0 |
| Point 3: never block (proxy to daemon IPC path) | 5,807 | 0 | 5,807 | 0 |
| Point 4: secrets | 9,846 | 6,104 | 3,742 | 0 |
| Point 5: upload | 9,869 | 4,418 | 5,451 | 0 |
| Point 6: tell the participant | 2,021 | 152 | 1,869 | 0 |
| Point 7: harnesses (connectors, launch, homes, setup, install) | 14,781 | 5,582 | 9,199 | 0 |
| End to end, local (points 1 to 4 together) | 6,346 | 2,729 | 3,617 | 0 |
| Chain format (the base the upload needs) | 3,597 | 148 | 3,449 | 0 |
| Cut part: research decoders | 17,264 | 0 | 0 | 17,264 |
| Cut part: separate daemon and its control API | 6,495 | 0 | 0 | 6,495 |
| Cut part: seal and signed status | 6,225 | 0 | 3,737 | 2,488 |
| Cut part: Rust stand-in of the upload server | 5,423 | 0 | 0 | 5,423 |
| Cut part: process termination and spawn gate | 5,145 | 0 | 0 | 5,145 |
| Cut part: live-run tooling (matrix, campaign) | 4,178 | 0 | 0 | 4,178 |
| Cut part: byte-for-byte goldens | 3,640 | 0 | 0 | 3,640 |
| Cut part: crash and budget campaigns (T25) | 2,881 | 0 | 0 | 2,881 |
| Cut part: event rules (workspace, repo, window, emails) | 2,536 | 0 | 1,893 | 643 |
| Cut part: device flow, login, GitHub check | 2,115 | 577 | 1,354 | 184 |
| Cut part: platform hardening (capabilities, sleep lock, machine facts) | 1,530 | 0 | 0 | 1,530 |
| Cut part: budgets and caps | 1,381 | 114 | 0 | 1,267 |
| Cut part: disk full and reserve | 1,310 | 0 | 0 | 1,310 |
| Cut part: connector pins | 1,249 | 0 | 449 | 800 |
| Cut part: seal command | 206 | 0 | 0 | 206 |
| Neither: release and repo tooling | 2,490 | 2,490 | 0 | 0 |
| Neither: test glue | 2,106 | 2,106 | 0 | 0 |
| Neither: CLI surface (commands, exit codes) | 782 | 0 | 782 | 0 |
| Neither: code rule (no harness names in core) | 378 | 378 | 0 | 0 |
| **Total** | **142,306** | **32,518** | **56,334** | **53,454** |

Points 1 to 7 plus the end-to-end and chain-format groups hold 74,972 lines (53 percent). Cut parts
hold 61,578 (43 percent). Tooling, glue and code rules hold 5,756 (4 percent).

### A4.2 `#[cfg(test)]` modules in `crates/*/src` (16,335 lines)

| Group | Lines | Verdict | Biggest modules |
|---|---|---|---|
| Point 1: daemon chain writer | 2,340 | rewrite | `goalcup-daemon/src/runtime/capture/tests.rs` 2,340 (also pins caps: `a_fresh_epoch_waits_while_max_live_producers_are_live_or_queued`, `capacity_exhaustion_uses_bounded_terminal_records`) |
| Point 1: proxy | 1,041 keep, 646 rewrite | keep / rewrite | `proxy/connect.rs` 284, `proxy/http.rs` 205, `proxy/preflight.rs` 170, `proxy/metadata.rs` 153; `model_host.rs` 646 |
| Point 3: outage windows (gaps) | 172 | keep | `proxy/outage.rs` 172 |
| Point 4: secrets | 1,134 rewrite, 304 keep | rewrite / keep | `redact/stream.rs` 1,063 (the email recognizer cases go), `redact.rs` 71; `run_secret.rs` 154, `proxy/headers.rs` 136 |
| Point 5: upload | 734 | keep | uploader `finish`, `heartbeat`, `lifecycle`, `local`; `upload/policy.rs`; `push_lines.rs`; `vitals.rs` |
| Point 6: participant | 1,361 | keep | `participant/tests.rs` 702, `run/mod.rs` 609, `commands/status.rs` 50 |
| Point 7: harnesses | 980 | keep | `home.rs` 248, `template.rs` 123, `launch.rs` 96, `doctor.rs` 85 |
| End to end: start and continue | 463 | rewrite | `commands/start/phases.rs` 202, `start/proxy.rs` 120 |
| Chain format | 362 | rewrite | `export.rs` 145, `capture.rs` 103 |
| Cut: disk full, reserve, pause, prune | 1,682 | delete | `run/watch.rs` 767, `run/reserve.rs` 422, `upload/prune.rs` 248, `run/pause.rs` 149, `platform/src/clone.rs` 96 |
| Cut: research decoders | 1,154 | delete | `capture_analysis/report.rs` 483, `wire.rs` 348 |
| Cut: daemon split (busy retry, control file) | 992 | delete | `retry.rs` 691, `config.rs` 190 |
| Cut: termination | 887 | delete | `protocol/src/process.rs` 251, `subreaper.rs` 263, `terminate/darwin.rs` 189 |
| Cut: platform hardening | 760 | delete | `terminal.rs` 190, `tracecap.rs` 177 |
| Cut: event rules (time-left tool, folder gate) | 509 | rules lane decides | `commands/mcp.rs` 201, `run/time_left.rs` 170 |
| Cut: pins | 418 | delete | `pins.rs` 230, `connector_dir.rs` 188 |
| Cut: seal | 378 | delete / rewrite | `commands/seal.rs` 197, `capture_analysis/reader.rs` 181 |
| Device login | 18 | keep | `account/device.rs` |

Note on `upload/prune.rs`: deleting acknowledged chain parts is the one disk rule a week-long run
may still need even with "no special handling" for a full disk. If the prune stays, keep its 248
lines of tests and cut only the reserve and pause.

## A5. The parts a cut would remove, test by test

| Part | Tests that guard it | Lines | Decision source |
|---|---|---|---|
| Proxy restart cap of five | `m2_proxy_integration.rs:2967-2998` (`the_recorder_is_restarted_at_most_five_times`), `:2999-3025` (count kept across `continue`); `run/mod.rs` cfg test `a_restarted_recorder_is_a_problem_line` | about 60 | Progress doc, Task 10 ("cap of 5 is an owner decision"). The cap is `MAX_RECORDER_RESTARTS = 5` at `crates/goalcup-cli/src/proxy/mod.rs:1019-1022`. Its doc says after the sixth death "the port is no longer served": the harness's `HTTPS_PROXY` then points at a dead port, so its model calls fail. That breaks point 3. Keep `a_dead_accept_loop_is_restarted_on_the_same_port` (2920-2966) and make it restart every time. |
| Native budgets (32 open streams, 1,024 files) | `capture_native.rs:1651-1709`, `:2385-2415`, `:2416-2479`, `:2480-2517`, `:2518-2552` | about 250 | Owner: raise or remove. Defaults at `crates/goalcup-daemon/src/native_capture.rs:272-284` (`max_open_streams: MAX_OPEN_STREAMS / 4`, `max_files: 1024`). |
| Entry caps (128 streams, 128 connections, 128 live producers) | `capture_ingest.rs:1555-1606` (`stream_limit_enforced`), `:2796-2830`; `m2_emitter.rs:802-825`, `:911-944` (`stream_and_connection_limits_disable_capture_instead_of_growing`), `:1022-1052`; `m2_proxy_integration.rs:1981-2147`; daemon `runtime/capture/tests.rs` (two rows) | about 500 | Owner: raise or remove. Constants at `crates/goalcup-protocol/src/capture.rs:57-66`. |
| 1,024 connection tasks, then a 503 | `m2_proxy_integration.rs` `excess_connections_are_refused_predictably` | about 60 (`m2_proxy_integration.rs:1846`) | Not in the owner list, but a 503 to the harness breaks point 3. `crates/goalcup-cli/src/proxy/mod.rs:109-121`. Its comment says it guards file descriptors, which is a real cost; say so, measure, and record a gap instead of refusing if possible. |
| IPC sender stops after ten failures | `m2_capture_sender.rs:1123-1160`, `recovery_budget_exhaustion_stays_incomplete` | about 100 | `MAX_CONNECTION_FAILURES = 10`, `crates/goalcup-cli/src/proxy/ingest.rs:100-104`. After ten, recording ends for the launch. That is a cap of our own on recording. |
| Finish budget | `goalcup-daemon/tests/full/capture_finish_budget.rs` | 1,267 | Simplify first. A fixed finish deadline can stay as one line in the E2E test. |
| Chain quota and disk reserve | `capture_failure_campaign.rs` (755 plus 152 support), `t25_disk_full.rs` (1,173), `platform/tests/it/disk.rs` (137), cfg tests in `run/reserve.rs`, `run/pause.rs`, `run/watch.rs` | about 4,500 | Owner: "Disk full: no special handling". |
| Server per-run limit | `server/worker/test/quota.test.ts` | 114 | Owner to decide. It guards storage cost, so keep it while the limit stays. |
| Seal and signed status | `capture_terminals.rs` 884, `seal_scan.rs` 206, `health_status.rs` 151, `signing_vectors.rs` 54, `sign.test.ts` 68, `v5_corpus.rs` 718, `m2_capture_integrity.rs` 3,737 | about 5,800 | No point needs tamper evidence. Risk: the upload and `fetch-run` may still read the seal to know a run is complete. Keep a small check that a rebuilt trace reads, whatever the seal becomes. |
| Termination and spawn gate | `process_kill.rs` 1,285, `terminate_sim.rs` 2,714, `capture_gate.rs` 519, `launcher_sweep.rs` 93, `process_handoff.rs` 147, support 387, cfg 887 | about 6,000 | Not a point. README P7 names termination, so the promise list must change first. Starting the proxy before the harness (already tested by `the_proxy_and_the_admitted_producer_precede_the_spawn_and_no_goal_is_passed` in `m2_cli_lifecycle.rs`) covers the record-from-the-start need. |
| Device approval and login | Worker `device.test.ts` 306, `device-unit.test.ts` 87, `ratelimit.test.ts` 184; `upload_server_device.rs` 529; `login.rs` 159; the login half of `account.rs` | about 1,600 | The upload needs some sign-in, so keep the Worker device tests while device login stays. Delete the Rust stand-in copy. The progress doc says device approve answered 404 after the 29 Oct deploy; that is a live check, not a test gap here (unverified). |
| GitHub repository check | `github.test.ts` 184; the repo half of `account.rs`; `upload_open.rs` repo rows | about 900 | Event rule, not a point. |
| Research decoders | 13 verifier test files (17,264 with `test_research_*` and `test_early_crosscheck.py`), cfg 1,154, verifier fixtures 2,182 | about 20,600 | Not a point: "record raw, interpret later". The participant's "Recording works: N model call(s)" line comes from the proxy count (`crates/goalcup-cli/src/commands/start/finish.rs:500-506`), not from a decoder. Risk: the CLI links the verifier for the key-fragment scan and `goalcup doctor` (`crates/goalcup-cli/Cargo.toml:31-33`); those need the reader only. |
| Byte-for-byte checks | `harness_contract_golden.rs` in cli (975), daemon (463), verifier (487); `m2_contract.rs` (1,715); `m2-report-bytes` fixtures; 2,110 lines of `harness-golden` fixtures | about 5,800 | Owner: byte match of subagent files is no longer required; no backward compatibility. The subagent byte match itself is not in this repo: it lives in the external live kit (`scripts/` has none; unverified where it lives). Keep `chain_golden.rs` and the Worker and Rust signing vectors only if signing stays. |
| Email masking | email rule at `crates/goalcup-daemon/src/redact.rs:280-287`; `redaction.rs` kind-list rows (`144`, `186`, `341`, `1324`, `1488`, `1616`) and the email-overlap rows; the email recognizer in `redact/stream.rs` and its cfg cases | about 300 to change | Owner option b. Warning: `redact.rs:280-283` says the email rule is what masks URL credentials today (`https://user:<token>@host`, a `git remote -v` line). Add the URL password rule and its test before the email rule goes, as the owner said. |
| Codex rate-limit masking | `connectors/codex-cli/harness.toml:109-117`; `redaction.rs:755-772`, `:935-968`; `capture_redaction_extra.rs`; `redaction-extra-patterns.json` golden | about 250 | Owner option b. Which Codex extras count as "rate-limit rules" is an owner call: `rate_limits`, `plan_type`, `credits` and `credits_remaining` are account facts; `x-codex-turn-state` and `x-codex-installation-id` may be tokens and should stay (unverified). |
| Rust stand-in of the upload server | `tests/common/upload_server/` (2,514) and its own tests (`upload_server.rs`, `_device.rs`, `_segments.rs`, 2,909) | 5,423 | It copies the Worker. Run the client against the real Worker instead (`upload_worker.rs` already does, with `wrangler dev`). Risk: those tests are `#[ignore]` and need Node; make them part of the gate. |
| Pins | `connector_pins.rs` 303, `pins_build_fails.rs` 86, `shipped_connectors.rs` 411, cfg 418 | about 1,200 | Tamper check, not a point. README P4 must change first. |
| Platform hardening | `unix_tracecap.rs`, `unix_sourcecap.rs`, `artifact_root_unix.rs`, `unix_snapshot_primitives.rs`, `awake.rs`, `machine.rs`; cfg 760 | about 2,300 | Not a point. The sleep lock keeps a laptop awake; a sleeping machine stops the harness too, so nothing goes unrecorded. |
| Live-run tooling | `test_matrix.py`, `test_linux_matrix.py`, `test_linux_check.py`, `test_campaign.py` | 4,178 | Tests of test runners. Keep the runners if they are used for the 2-hour run, but their unit tests are not needed by Friday. The 2-hour run report does not exist on any branch yet. |

## A6. Gaps: points no test proves today

| Gap | Point | Why it matters |
|---|---|---|
| Hundreds of subagents at once | 2 | The largest native test is `old_files_are_skipped_and_the_new_one_found_among_5000` (old files, one new). `two_new_session_files_are_both_recorded` is the only multi-file growth test. Nothing runs past 32 open streams, 128 entry streams or 256 open files (macOS default). |
| Recorder dies more than five times | 3 | Today the sixth death leaves the port unserved. |
| URL password masked | 4 | No rule or test of its own; today it rides on the email rule. |
| Client to real Worker to `fetch-run`, ordered by ID and time | 5 | `upload_worker.rs` (ignored by default) and `test_fetch_run.py` (fake bucket) never meet. Nothing checks that the rebuilt story equals the local one. |
| What the participant is told, at scale | 6 | Finish lines are tested with tiny counts. Nothing checks the wording after a gap or a restart in one place. |
| One offline case per shipped harness | 7 | The kit has two synthetic cases. `full/connector_contract.rs` runs every connector once with a stand-in; that is the real per-harness check. Real harness runs live outside the repo (external kit; unverified). OpenCode and OpenClaw exist only on `origin/opencode-openclaw`. |
| Subagents asked for in the prompt | 2, 7 | Only live runs can do this. The 29 Oct table shows Hermes 0.19.0 delegation failing (a harness defect) and no measured Hermes subagent run. |

## A7. Recommendations

### A7.1 First, change the promise list

Replace README P1 to P7 with R1 to R7 (section A3) in the same PR that deletes tests. Otherwise
README.md:278 and AGENTS.md:101 forbid the deletes. What could go wrong: a reviewer reads the old
rule and blocks the PR. Fix: one line in the PR saying the owner decision of 29 Oct replaces it.

### A7.2 Keep (about 37,000 lines)

The relay suites (`m2_http_relay.rs`, `m2_websocket_relay.rs`, `m2_tls_connect.rs` and their
support), `m2_synthetic_vertical.rs` and its support, the streaming redaction suites
(`capture_redaction.rs`, run secrets, `hermes_offline_key.rs`, `ps_env.rs`), `upload_chunks.rs`,
`upload_worker.rs`, the Worker's `integration.test.ts`, `supabase.test.ts` and `segments.test.ts`,
`test_fetch_run.py`, `test_install.py`, the connector contract (static and full), the kit, the
loader, conformance and template tests, `native_silence.rs`, the release tooling tests and the
`#[cfg(test)]` modules listed as keep in A4.2.

### A7.3 Rewrite (about 61,000 lines now, target about 30,000)

- Cap tests: turn each into "past the old number, everything is still recorded, or a gap is
  noted and the harness goes on". Never assert a cap the owner removed.
- `redaction.rs`: drop the email rule and the Codex rate-limit rows; add URL password rows; update
  the ordered kind list. Keep every secret row.
- `capture_native.rs`: keep discovery, safety (symlink, FIFO) and the finish tail; replace the
  1,024-file and 32-stream rows with one many-files test.
- `m2_emitter.rs`, `m2_capture_sender.rs`, `m2_ipc_compose.rs`, `capture_ingest.rs`,
  `capture_lifecycle.rs`: keep the never-block and loss-is-a-gap rows; drop recovery-epoch
  bookkeeping if the daemon is folded into the CLI (another lane decides).
- `upload_e2e.rs`, `upload_open.rs`, `restarts.rs`: move the rows worth keeping onto the real
  Worker; drop GitHub, start-window and practice rows if those rules leave the client.
- `harness_schema.rs` (4,451, 114 rows): keep the rows that stop a base URL, a provider key in
  argv or env, and a write into an app-wide home. The rest are shape checks.
- `prep.rs`, `start_launch.rs`, `status_views.rs`, `preflight_doctor.rs`: drop rows for reserve,
  WSL host filesystems, sleep lock and signatures if those parts go.

### A7.4 Delete (about 60,000 lines, with the code they guard)

Every file marked delete in the table in section A9. Biggest: the verifier decoder suites, the
upload-server stand-in, the termination suites, the daemon control API suites, the goldens, the
T25 campaigns and the live-run tooling tests.

### A7.5 The minimum test set that proves the seven points end to end

About 27,000 lines of existing tests (after the rewrites above) plus six new tests.

| Point | Fast tests (gate) | End to end |
|---|---|---|
| 1 | `m2_http_relay.rs` (framing matrix, `only_a_request_on_a_hinted_path_counts_as_a_model_call`, `every_request_on_an_opened_host_is_recorded` from `m2_proxy_integration.rs`), `m2_websocket_relay.rs`, `m2_tls_connect.rs` admission rows, `model_host_from.rs` | E2E below |
| 2 | `capture_native.rs` core rows, `a_compaction_mid_run_starts_a_second_session_file_and_both_are_recorded` (`hermes_offline_key.rs`), **new N1** | E2E below |
| 3 | `capture_disabled_does_not_block_forwarding`, `queue_pressure_recovers_at_next_http_request` (`m2_http_relay.rs`), `sender_never_waits_for_local_ack`, `queue_full_latches_without_blocking_relay` (`m2_capture_sender.rs`), `stop_with_active_stream_and_blocked_sink_meets_one_deadline`, `an_upstream_outage_is_one_window_closed_by_the_next_reach`, `a_server_refusing_every_call_for_a_while_never_stops_the_run_and_gets_every_record_once` (`upload_chunks.rs`), `proxy/outage.rs` cfg tests, **new N2** | E2E below |
| 4 | `capture_redaction.rs`, `capture_redaction_run_secrets.rs`, rewritten `redaction.rs`, `hermes_offline_key.rs`, `credentials_reach_upstream_but_never_capture`, `no_connector_passes_a_provider_key`, `proxy/headers.rs` cfg tests, `ps_env.rs`, **new N3** | E2E plants one secret of each rule |
| 5 | `upload_chunks.rs`, Worker `integration.test.ts`, `supabase.test.ts`, `segments.test.ts`, `test_fetch_run.py` | **new N4** |
| 6 | `participant/tests.rs`, `native_silence.rs`, the "Recording works" rows in `model_host_from.rs`, a slim `status_views.rs` | E2E checks the finish lines |
| 7 | `connector_contract.rs` (static), `full/connector_contract.rs`, `harness_contract_kit.rs`, `harness_loader.rs`, `harness_conformance.rs`, `connector_template.rs`, `test_install.py` | **new N5**, **N6** |

New tests:

- **N1, many subagents** (daemon, about 150 lines): 2,000 session files, 400 growing at once, under
  `ulimit -n 256`. Every byte of every file is recorded, or a named gap per file. Fails today by
  design (32, 128, 1,024 caps).
- **N2, the recorder dies ten times** (proxy, about 60 lines): restarted on the same port every
  time; the harness's calls after each restart succeed.
- **N3, URL password** (redaction, about 60 lines): `https://user:pass@host/x`, a `git remote -v`
  line, a percent-encoded form, split at every byte. A plain email stays in clear.
- **N4, the server rebuilds the run** (E2E, about 300 lines on top of `upload_worker.rs`): real
  client, real Worker, Worker stopped and started mid-run, then `fetch-run.sh`; the rebuilt chain
  equals the local one, and every wire call and every native file path (its ID) is present and
  in time order. Make it part of the full gate, not `#[ignore]`.
- **N5, one week in minutes** (E2E, about 400 lines, extends `m2_synthetic_vertical.rs`): the
  controlled workload makes HTTP, SSE and WebSocket calls to two providers, starts 300 subagent
  files, prints a secret of each rule, sees one upstream outage and two recorder deaths, and
  uploads. Assert points 1 to 6 in one run, plus the finish lines.
- **N6, live runs** (not in the gate): Claude Code, Codex and Hermes (plus any harness on the
  supported list), run the way people run them, with subagents asked for in the prompt, never
  forced by config. Pass: chain has every subagent file, decoded or proxy call count is explained,
  no secret. These need keys and live outside the gate.

## A8. Blind spots and what could go wrong

| Recommendation | What could go wrong | Mitigation |
|---|---|---|
| Delete the decoder suites | A judge or researcher needs decoded calls after the event; the key-fragment scan and `doctor` use the verifier reader | Keep the decoders on a branch; keep only the reader the CLI uses, with a small test |
| Delete termination tests | A harness that forks and leaves keeps running after the seal and its later calls are lost or confuse the record | Proxy keeps recording until the process tree is gone; one E2E row checks a child that outlives the root |
| Remove caps | File handle or memory exhaustion on a long run | N1 under `ulimit -n 256`; measure, then set a limit that names the real cost (descriptors) and records a gap instead of refusing |
| Delete the stand-in and test against the Worker | Gate needs Node and `wrangler dev`; slower, flakier | Keep the in-process Worker rig (`helpers/inproc.ts`) for fast rows; one `wrangler dev` run in the full gate |
| Drop the email rule | URL credentials leak, since the email rule catches them today | Land N3 and the URL password rule first, as the owner said |
| Delete seal tests | `fetch-run` or the Worker may depend on the seal to close a run | Decide the seal's fate in the seal lane; N4 tests whatever close remains |
| Rewrite rather than delete the IPC tests | If the daemon is folded into the CLI, this rewrite is wasted | Wait for the daemon decision before rewriting `m2_emitter.rs`, `m2_capture_sender.rs`, `m2_ipc_compose.rs` |
| Rely on live runs for point 7 | Keys, cost and harness defects (Hermes 0.19.0 delegation) make them rare | Record each run's result; the 2-hour run report does not exist yet, so do not claim it |
| My grouping | One primary group per file hides mixed files (for example `capture_lifecycle.rs`) | Notes in the table name the mixed rows; line ranges were found by test name and are approximate |

Unverified: where the external live kit lives and what its byte-for-byte check does; whether the
Worker's device approval 404 seen on 29 Oct is fixed; which Codex extra patterns the owner means by
"rate-limit rules".

# Part B: the gate

Read-only review of how Goal Cup Proxy tests itself, at origin/main 4b226de2. I did not run the gate.
Every time below comes from a commit message or a doc. Where nothing was recorded, I say "not recorded".

## B1. What the gate is

| Part (path) | Lines | What it does |
|---|---|---|
| `scripts/gate.sh` | 113 (script) | The gate. `fast` before every commit, `full` before a release. Last line `gate <tier>: PASS in <N>s` or `FAIL` (scripts/gate.sh:1-21). |
| `scripts/test_gate.py` | 226 (tests, 16 test methods) | Unit tests of gate.sh. |
| `.config/nextest.toml` | 61 (config) | One process per test; slow flag at 30 s, kill at 180 s; two long tests start first; `serial` and `connector-runs` groups of one thread (.config/nextest.toml:1-61). |
| `scripts/check-version.py` | 206 (script) + 128 (tests) | Fails if the version is written anywhere but the workspace Cargo.toml. First gate step. |
| `scripts/check-links.py` | 1349 (script) + 1672 (tests) | Relative Markdown link and anchor check. CI and full gate. |
| `scripts/ci-scope.py` | 90 (script) + 134 (tests) | Prints `rust=true/false`: skips Rust in CI only when every changed path is Markdown docs (scripts/ci-scope.py:1-18, 33-46). |
| `.github/workflows/ci.yml` | 93 | Light Linux CI, explicitly "NOT the gate" (ci.yml:1-21). |
| `.github/workflows/linux-x86_64.yml` | 63 | Manual only: `linux-check.sh` in Docker plus a harness smoke. |
| `.github/workflows/release.yml` | 120 | Builds, packages and smoke-tests release tarballs. |
| `scripts/linux-check.sh` | 106 | "The Linux half of the gate", in Docker with ext4 and XFS loop volumes (scripts/linux-check.sh:1-16). |

Test volume the gate drives (attribute counts of `#[test]` and `#[tokio::test]`, src unit tests included; macro-generated
tests such as the `shards!` redaction parts are not counted, so the real number is higher):
goalcup-cli 1013 (31 ignored), goalcup-daemon 540 (1 ignored), goalcup-protocol 369, goalcup-verifier 378 (1 ignored),
goalcup-platform 101. About 2,400 in all. The Worker has 276 tests (progress doc, "276 of 276 Worker tests passed").
The scripts have 577 Python test methods in 20 files (10,393 test lines).

## B2. Gate steps, in order

### Fast tier (scripts/gate.sh:61-96)

| # | Step | Where | Time recorded |
|---|---|---|---|
| 0 | Refuse to run as root unless `GOALCUP_GATE_ALLOW_ROOT=1` (permission tests fail as root) | gate.sh:32-39 | instant |
| 1 | `python3 scripts/check-version.py` | gate.sh:61 | not recorded (seconds) |
| 2 | `cargo build --offline --workspace` (tests spawn the sibling `goalcup-daemon`, so build first) | gate.sh:63-65 | see totals |
| 3 | `cargo fmt --all --check` | gate.sh:66 | not recorded |
| 4 | `cargo clippy --offline --workspace --all-targets -- -D warnings` | gate.sh:67 | not recorded |
| 5 | `cargo nextest run --offline --workspace -E "not ((binary(full) & not test(/^connector_contract::/)) \| binary(m2_cli_lifecycle))"`. Probes the filter first; if a binary was renamed it warns and runs every test | gate.sh:69-83 | see totals |
| 6 | `cargo test --offline --workspace --doc` (nextest skips doctests; one `compile_fail` doctest in goalcup-protocol guards a promise) | gate.sh:95-96 | not recorded |

Without cargo-nextest the gate falls back to plain `cargo test` for every tier (gate.sh:97-105).

### Full tier (adds to fast; scripts/gate.sh:54-58, 84-111)

| # | Step | Where | Time recorded |
|---|---|---|---|
| 0b | Fail early if `server/worker/node_modules` is missing | gate.sh:54-58 | instant |
| 5' | `cargo nextest run --offline --workspace` (everything not ignored, incl. `full` binaries and `m2_cli_lifecycle`) | gate.sh:85 | `m2_cli_lifecycle` about 100 to 105 s (gate.sh:71-72; commit d9836dcf) |
| 7 | Ignored end-to-end tests: `m2_synthetic_vertical` (8) and `capture_failure_campaign` (6), `--run-ignored ignored-only` | gate.sh:86-89 | about 4.5 minutes (gate.sh:12-14; commit cffa1a38) |
| 8 | Ignored `upload_worker` tests: the real Worker under `wrangler dev` | gate.sh:90-93 | not recorded |
| 9 | Doctests | gate.sh:96 | not recorded |
| 10 | `npm --prefix server/worker test` = `npm run typecheck && node --test --test-timeout=120000 "test/**/*.test.ts"` (server/worker/package.json:13) | gate.sh:108 | not recorded |
| 11 | `python3 -m unittest discover -s scripts` (all 20 script test files) | gate.sh:109 | not recorded |
| 12 | `python3 scripts/check-links.py --root .` | gate.sh:110 | not recorded |

Not in either tier, run by hand: other ignored tests that need a real Hermes or a disk image (gate.sh:16-17), e.g.
`t25_disk_full`; `scripts/linux-check.sh` (Docker, before a release, README "Releasing"); the live kit runs.

### CI (not the gate)

`ci.yml` runs on every PR into main, push to main and manual start, one `ubuntu-24.04` job, 20-minute timeout
(ci.yml:25-47). Steps: scope (ci-scope.py), four script unit tests (`test_ci_scope`, `test_check_links`,
`test_check_connector`, `test_codeowners`), the link check, then only if `rust=true`: build, fmt, clippy, and
`cargo test --workspace --exclude goalcup-daemon --exclude goalcup-cli` (ci.yml:60-93). The daemon and CLI suites
are kept out because "on a 2-core Linux runner they hang or kill the VM" (ci.yml:15-16; docs/OPERATING.md:530-545).
CI time per run: not recorded.

### Manual Linux CI

`linux-x86_64.yml`: job `linux-check` (150-minute timeout) runs `sh scripts/linux-check.sh` with
`LINUX_CHECK_HARNESSES=1`; job `harness-smoke` (45-minute timeout) builds release binaries and runs
`scripts/linux-harness-smoke.sh` (linux-x86_64.yml:25-63). Actual run times: not recorded.

## B3. Time per step and total

| Measurement | Value | Date, machine | Source |
|---|---|---|---|
| Fast gate, warm median | 54 s | 12 Oct, quiet machine | commit ba10ccca message |
| Fast gate, warm | about 2.5 min | undated | docs/HARNESS-ONBOARDING.md:246-247 |
| Fast gate test time, before and after speed-ups (redaction shards, daemon opt-level 1, each kit case once) | about 335 s, then about 190 s; slowest test 99 s to about 10 s | 21 Oct | docs/research/SIMPLIFICATION-2026-10-21.md:9-12, 18-20, 31 |
| Fast gate build plus nextest steps by daemon opt-level 0 / 1 / 2 | 364 s / 295 s / 326 s | 21 Oct, 4 vCPU | Cargo.toml:55-62 |
| "Sandbox gate on main PASS" | 306 s | 28 Oct (tier not stated, likely fast) | docs/plans/2026-10-29-record-everything-progress.md:47 |
| "Gate PASS ... with Worker tests" (Task 3) | 146 s | 28 Oct (tier not stated) | same file:49 |
| `check-connector.sh` | 1 to 1.5 min | undated | docs/HARNESS-ONBOARDING.md:246-247 |
| `m2_cli_lifecycle` binary (full only) | about 100 to 105 s | 12 Oct | gate.sh:71-72; commit d9836dcf |
| Ignored e2e modules (full only) | about 4.5 min | 13 Oct | gate.sh:12-14; commit cffa1a38 |
| Release build of the three binaries | 1 min 48 s | Oct, sandbox | origin/capture/plan-hardening:docs/research/results/2026-10-vercel-gateway.md:152 |
| Full gate total | not recorded anywhere I found | | My estimate, unverified: about 10 to 15 min warm on 4 vCPU (fast about 3 to 5 min, then about 4.5 min serial e2e, plus upload_worker, Worker and script tests). |

Why the fast gate is slow: the P1 redaction loops "use over half of it" (SIMPLIFICATION-2026-10-21.md:19-20);
two long tests are started first so they overlap the rest (.config/nextest.toml:10-19); the regex crates are
built at opt-level 3 because unoptimised they made redaction tests 4 to 9 times slower and two passed the 180 s
kill on a 4-vCPU runner (Cargo.toml:36-53; commit 39ba14fa).

## B4. Known flaky tests and root causes

### Open

| Test | Evidence | Root cause | Status |
|---|---|---|---|
| `an_upstream_outage_is_one_window_closed_by_the_next_reach` (crates/goalcup-cli/tests/it/m2_proxy_integration.rs:2853-2915) | Failed on the first gate run of the merged #289 head (28 Oct); passed 5 of 5 locally and on the rerun (progress doc:51) | Suspected, unverified (the failure text was not recorded): the test binds `127.0.0.1:0`, reads the port and drops the listener (lines 2865-2868), then expects three refused connects answered `502`. Any other test process (nextest runs hundreds in parallel, each binding port 0) can be handed that port and listen on it. The window is wide: `localhost` is an auth host, which is retried until the admission deadline, and the rig sets `admission_timeout` 5 s and `connect_attempt_timeout` 2 s (crates/goalcup-cli/tests/common/m2_proxy_support/mod.rs:836-837; retry rule in m2_tls_connect.rs:302-305). The same test flaked once before, on 13 Oct, from a real product race (wall time stamped before the tracker lock), fixed in commit a2e92f7f. | Not fixed |
| The port reservation race in general | Same pattern | Bind port 0, drop, reuse the number later. Sites: crates/goalcup-cli/src/proxy/connect.rs:1315-1319 (`free_port`, then re-binds after 700 ms: another process can take it and the `bind().unwrap()` panics); src/proxy/preflight.rs:256-258; tests/it/m2_tls_connect.rs:274-276, 307-309; tests/it/login.rs:136-138; tests/it/preflight_doctor.rs:391-393; tests/full/upload_e2e.rs:113-115; tests/full/upload_worker.rs:191-197 (port for `wrangler dev`); tests/it/hermes_offline_key.rs:537; tests/m2_cli_lifecycle.rs:810; crates/goalcup-daemon/tests/it/control_api.rs:896-898, 1319-1321. | Not fixed |
| `connector-runs` group (kit and dynamic connector contract) | .config/nextest.toml:45-52 | The group may no longer be needed after the `@GO@` handshake, but that "rests on five of five runs with the group off under load, which is not proof". Kept serial. | Mitigated, not proven |
| Tight wall-clock upper bounds | about 30 `elapsed() <` checks in crates (rg) | Fixed limits that assume an idle machine. Tightest: crates/goalcup-cli/tests/it/m2_capture_sender.rs:1795 (< 50 ms), :1279 (< 200 ms), :382 (< 250 ms), :285 (< 1 s); tests/it/upload_chunks.rs:1728 (< 1 s); tests/it/m2_tls_connect.rs:292 (< 2 s); tests/it/daemon_client.rs:984 (< 2 s); src/run/watch.rs:1331, 1342 (< 2 s); src/proxy/preflight.rs:267 (< 2 s). No failure of these is recorded; risk only. | Not observed failing |

"Sergio-style" wall-clock tests: there is no test of that name in this repo. The `sergio-main-local-db` wall-clock
flake belongs to the other repository (agentcommunity_page). The tight bounds above are this repo's nearest analogue.

### Fixed (history, to show the pattern)

| Commit (date) | Test or area | Root cause |
|---|---|---|
| a2e92f7f (13 Oct) | `an_upstream_outage_...`: `first_failure <= last_failure` failed | Product race: wall time read before the tracker lock; now read under the lock |
| 4134b86b (14 Oct, PR #92) | Concurrent outage-window test | Test bound one too small: 32 successes against 32 kept windows (11 of 400 runs) |
| 5badeda8, ea8a0873 (12-13 Oct) | `two_concurrent_starts_end_with_exactly_one_daemon` | Product races: registry file create on APFS gave ENOENT (8 of 10); two `mkdir` of one capture home gave EEXIST |
| 39ba14fa, 77b5fea3 (14 Oct) | Redaction property tests; subreaper sweep bound | Unoptimised regex passed the 180 s kill on 4 vCPU; sweep host ran out of children under load |
| 4234da45 (15 Oct, PR #113) | Sleep lock (Linux) | Returned Ok before the hold was proven |
| e13e1fb8 (16 Oct, PR #194) | Four tests | Shared process-wide signal handlers under `cargo test`; ext4 directory order (1 fs in 65); fake server reset caused macOS EINVAL; 1-second run refused a chunk with 503 |
| f7d3f838 (16 Oct, PR #196) | Upload-server incomplete run | Whole-second server clock; chunk after the run ended |
| d56e7e62 (17 Oct, PR #216) | Held-open seal test | Finish started while curl was still in CONNECT/TLS |
| 5c660a93 (23 Oct) | matrix.sh seal timing; allowlist probes | Whole-second clock vs exact stamp, 1 run in 5 under load; probes reached real staging |
| 6c41fed3 (27 Oct, PR #285) | `a_running_child_has_both_digests...` | vfork: `/proc/<pid>/exe` read before exec finished; leaked a 30 s sleep (nextest LEAK) |
| f707b014 (28 Oct, PR #287) | Seal registry reopen | axum connection task still held redb's lock after stop (4 of 160 runs at -j 16) |
| fce86a4 (28 Oct) | Kit test (not a flake) | Kit matcher counted the new header stream as a second request (progress doc:57) |

Common root causes: (1) shared OS resources between parallel test processes (ports, file locks, signal handlers);
(2) whole-second or wall clocks compared with exact stamps; (3) fixed time limits on a loaded 4-vCPU machine;
(4) assumptions about the order of concurrent events. About one in three of the fixed ones was a real product
race that a test found (outage stamp order, concurrent starts, sleep lock).

Other known gate constraints: the gate refuses root (gate.sh:32-39). Whether the Vercel Sandbox runs as root is
unverified; if it does, `GOALCUP_GATE_ALLOW_ROOT=1` is needed and the permission tests fail there by design.
On Linux the gate also needs `nc`, procps `ps`, `*.localhost` on loopback, IPv6 loopback and TMPDIR on ext4 or
XFS (README.md:259-265).

## B5. A minimal gate for the cut version

Aim: one command, a few minutes, and the checks that guard the seven points. Tests for code that is cut go with
that code; the gate script itself barely changes.

| Keep / change / cut | Step | Why (points) | What could go wrong |
|---|---|---|---|
| Keep | `cargo build --workspace` | Tests drive the built binaries | none |
| Keep | `cargo fmt --check`, `cargo clippy -D warnings` | Cheap; clippy caught Linux-only issues before (commit 84a26baf) | none |
| Keep | `cargo nextest run` without the slow tier (same filter) | Points 1 to 5: redaction (4), capture (1, 2), never block (3) | A cut that renames `full` or `m2_cli_lifecycle` makes the fast tier run everything; gate.sh already warns (gate.sh:80-81) |
| Keep, release only | Ignored e2e `m2_synthetic_vertical`, `capture_failure_campaign`, and `upload_worker` | The only full-pipeline checks of daemon, proxy, seal, verifier and the real Worker (commit cffa1a38). This week protocol, verifier, Worker and client change together, so these matter more, not less (point 5) | 4.5 min plus; `upload_worker` needs `npm ci` and uses the port race pattern |
| Keep, when server/worker or the protocol changes | `npm --prefix server/worker test` | Point 5: the Worker is the other half of any protocol change | Forgetting it when only Rust changed but the wire changed; simplest: always run it in the full tier |
| Keep | One doctest step | Only one `compile_fail` doctest guards a promise | If that doctest's module is cut, drop the step |
| Change | Fix the port race once: never drop a reserved port. Use a fixed port outside the ephemeral range (for example port 1, which bind(0) never hands out on Linux or macOS) for "nothing listens here", or keep the reserving socket open until the test ends | Removes the one open flake and about 12 sites | Port 1 could be in use on an odd machine; a held-open socket in LISTEN state accepts instead of refusing, so it must stay bound but not listening. Verify on macOS and Linux |
| Change | Loosen the sub-second upper bounds (50 to 250 ms) to seconds, or assert ordering instead of duration | Same flake class as the fixed ones | A looser bound may miss a real slowdown; the intent (no wait) can be checked with an event, not a clock |
| Cut from gate (keep in CI if wanted) | `check-links.py` (1349 + 1672 lines), the `unittest discover` of all script tests | Docs links and research/campaign scripts serve none of the seven points directly | Broken doc links; untested scripts if those scripts survive the cut. If the scripts lane cuts the matrix, campaign, research, fetch-run and early cross-check scripts, about 345 of the 577 Python tests go with them |
| Cut or keep (small) | `check-version.py` (206 + 128 lines) | Guards the release process, not a point | A second version string slips in; low risk for one release |
| Cut | `ci-scope.py` (90 + 134 lines) and the scope step | Only saves CI minutes on docs-only PRs | Docs-only PRs run the Rust smoke; a few minutes of runner time |
| Keep, by hand | One live run per supported harness that asks for subagents in the prompt (Claude Code, Codex, Hermes), on Linux in the Vercel Sandbox | Points 1, 2, 6, 7 can only be proven live. The gate has never covered this | Needs keys and money; Hermes 0.19 delegation fails (progress doc:74); the 2-hour run report does not exist on any branch yet |

Suggested shape for the week, with no new script:
- Every commit: `sh scripts/gate.sh fast` (about 3 to 5 minutes warm on 4 vCPU, from the records above).
- Before the Friday tag and before the 2-hour run: `sh scripts/gate.sh full` on the Linux sandbox and on a Mac,
  then the live runs.
- CI unchanged apart from dropping the scope step, if the scripts lane agrees.

Blind spots:
- No recorded time for the full tier, CI, the Worker tests or linux-check. Measure once before relying on any budget.
- The fast tier was timed on a quiet machine (54 s) and on a loaded sandbox (306 s): a 5x spread. Gate time on
  the shared 4-CPU machine will be noisy.
- The open flake's real cause is a guess; the failing output was not saved. Save nextest's JUnit or log output
  on the next failure.
- Raising the stream and file caps (owner decision) will add tests that open hundreds of files; the macOS
  default of 256 open files per process could make new tests flaky in the same way.

# Appendix: every test file

## A9. Every test file

Lines are `wc -l` of the test file. "Protects" is the primary group. Verdict is keep, rewrite or
delete (delete means with the part it guards).

| File | Lines | Protects | Verdict | Note |
|---|---|---|---|---|
| `cli/tests/common/capture_campaign_support/mod.rs` | 337 | cut: crash and budget campaigns | delete |  |
| `cli/tests/common/capture_crash_support/mod.rs` | 172 | cut: crash and budget campaigns | delete |  |
| `cli/tests/common/capture_failure_support/mod.rs` | 152 | cut: crash and budget campaigns | delete |  |
| `cli/tests/common/m2_cli_support/mod.rs` | 1673 | E2E (P1-P4 local) | rewrite |  |
| `cli/tests/common/m2_common/mod.rs` | 240 | P1 wire capture | keep |  |
| `cli/tests/common/m2_http_support/mod.rs` | 486 | P1 wire capture | keep |  |
| `cli/tests/common/m2_profile_support/mod.rs` | 171 | P7 harnesses | keep |  |
| `cli/tests/common/m2_proxy_support/mod.rs` | 1300 | P1 wire capture | rewrite |  |
| `cli/tests/common/m2_sender_support/mod.rs` | 693 | P3 never block (IPC path) | rewrite |  |
| `cli/tests/common/m2_tls_support/mod.rs` | 525 | P1 wire capture | keep |  |
| `cli/tests/common/m2_vertical_support/endpoint.rs` | 692 | E2E (P1-P4 local) | keep |  |
| `cli/tests/common/m2_vertical_support/mod.rs` | 800 | E2E (P1-P4 local) | keep |  |
| `cli/tests/common/m2_vertical_support/wire.rs` | 356 | E2E (P1-P4 local) | keep |  |
| `cli/tests/common/m2_ws_support/mod.rs` | 783 | P1 wire capture | keep |  |
| `cli/tests/common/mod.rs` | 35 | N glue | keep |  |
| `cli/tests/common/real_home_guard.rs` | 171 | P7 harnesses | keep |  |
| `cli/tests/common/secret_support/mod.rs` | 9 | P4 secrets | keep |  |
| `cli/tests/common/support/mod.rs` | 960 | N glue | keep |  |
| `cli/tests/common/upload_server/handlers.rs` | 1506 | cut: upload-server stand-in | delete |  |
| `cli/tests/common/upload_server/mod.rs` | 603 | cut: upload-server stand-in | delete |  |
| `cli/tests/common/upload_server/state.rs` | 405 | cut: upload-server stand-in | delete |  |
| `cli/tests/contract_gate.rs` | 782 | N CLI surface (commands, exit codes) | rewrite | exit codes, command tree, control.json; keep commands and exit codes only |
| `cli/tests/env_resolution.rs` | 217 | N glue | keep |  |
| `cli/tests/full/capture_crash_campaign.rs` | 1465 | cut: crash and budget campaigns | delete | T25 crash rows |
| `cli/tests/full/capture_failure_campaign.rs` | 755 | cut: crash and budget campaigns | delete | T25 quota rows |
| `cli/tests/full/connector_contract.rs` | 463 | P7 harnesses | keep | README P7; every connector once with a stand-in |
| `cli/tests/full/launcher_sweep.rs` | 93 | cut: termination and spawn gate | delete |  |
| `cli/tests/full/m2_synthetic_vertical.rs` | 881 | E2E (P1-P4 local) | keep | best base for the new end-to-end test |
| `cli/tests/full/main.rs` | 26 | N glue | keep |  |
| `cli/tests/full/per_run_lifecycle.rs` | 366 | P7 harnesses | keep |  |
| `cli/tests/full/run_scoped_daemon.rs` | 897 | cut: separate daemon and control API | delete | daemon start and exit |
| `cli/tests/full/t25_disk_full.rs` | 1173 | cut: disk full and reserve | delete | literal ENOSPC; owner: no special handling |
| `cli/tests/full/upload_e2e.rs` | 2518 | P5 upload | rewrite | same story against the Rust stand-in; move the rows worth keeping to upload_worker |
| `cli/tests/full/upload_worker.rs` | 456 | P5 upload | keep | real Worker under wrangler dev; make it the main upload test |
| `cli/tests/it/account.rs` | 1195 | cut: device flow, login, GitHub | rewrite | device login (keep a little) and GitHub repo check (event rule) |
| `cli/tests/it/ca_bundle.rs` | 88 | P1 wire capture | keep |  |
| `cli/tests/it/capture_gate.rs` | 519 | cut: termination and spawn gate | delete | spawn gate (D40) and ending a released child |
| `cli/tests/it/capture_only_refusal.rs` | 212 | P7 harnesses | rewrite |  |
| `cli/tests/it/connector_contract.rs` | 994 | P7 harnesses | keep | README P1, P3, P4; no base URL, no provider key |
| `cli/tests/it/daemon_client.rs` | 1248 | cut: separate daemon and control API | delete | CLI to daemon client and busy retry |
| `cli/tests/it/dependency_bans.rs` | 16 | cut: separate daemon and control API | delete |  |
| `cli/tests/it/harness_contract_golden.rs` | 975 | cut: byte-for-byte goldens | delete | byte-for-byte goldens of internal facts |
| `cli/tests/it/harness_contract_kit.rs` | 805 | P7 harnesses | keep | only two synthetic cases (controlled-workload, synthetic-per-run) |
| `cli/tests/it/harness_launch_plan.rs` | 782 | P7 harnesses | rewrite |  |
| `cli/tests/it/harness_version_probe.rs` | 195 | P7 harnesses | rewrite |  |
| `cli/tests/it/hermes_offline_key.rs` | 626 | P4 secrets | keep | README P1; key never in trace or upload; also a second session file (point 2) |
| `cli/tests/it/login.rs` | 159 | cut: device flow, login, GitHub | rewrite |  |
| `cli/tests/it/m2_capture_sender.rs` | 1991 | P3 never block (IPC path) | rewrite | never waits for the daemon (point 3, keep); 1123-1160 pins the ten-failure stop |
| `cli/tests/it/m2_emitter.rs` | 1517 | P3 never block (IPC path) | rewrite | loss chaining (point 3); rows 802-825, 911-944, 1022-1052 pin caps that switch capture off |
| `cli/tests/it/m2_http_relay.rs` | 2249 | P1 wire capture | keep |  |
| `cli/tests/it/m2_ipc_compose.rs` | 1606 | P3 never block (IPC path) | rewrite | proxy to daemon over TCP; shrinks or goes if the daemon is folded in |
| `cli/tests/it/m2_proxy_integration.rs` | 3025 | P1 wire capture | rewrite | drop the restart-cap rows (2967-3025); rewrite the cap rows (1981-2147) to the no-cap rule |
| `cli/tests/it/m2_tls_connect.rs` | 1275 | P1 wire capture | keep |  |
| `cli/tests/it/m2_websocket_relay.rs` | 2074 | P1 wire capture | keep |  |
| `cli/tests/it/main.rs` | 52 | N glue | keep |  |
| `cli/tests/it/model_host_from.rs` | 793 | P1 wire capture | rewrite | which host is recorded; also the "Recording works" lines (point 6) |
| `cli/tests/it/native_silence.rs` | 152 | P6 tell participant | keep |  |
| `cli/tests/it/per_run_home.rs` | 438 | P7 harnesses | keep |  |
| `cli/tests/it/preflight_doctor.rs` | 478 | P7 harnesses | rewrite |  |
| `cli/tests/it/prep.rs` | 1600 | P7 harnesses | rewrite | setup flow; keep the steps a participant needs |
| `cli/tests/it/process_handoff.rs` | 147 | cut: termination and spawn gate | delete |  |
| `cli/tests/it/restarts.rs` | 497 | P5 upload | rewrite | platform stops and continue; keep one row |
| `cli/tests/it/seal_scan.rs` | 206 | cut: seal command | delete |  |
| `cli/tests/it/start_launch.rs` | 1481 | P7 harnesses | rewrite | launch, terminal, signals; drop reserve and WSL rows if those parts go |
| `cli/tests/it/status_views.rs` | 1869 | P6 tell participant | rewrite | status output (point 6); drop the signature rows |
| `cli/tests/it/upload_chunks.rs` | 1736 | P5 upload | keep | resume, retry, a refusing server never stops the run |
| `cli/tests/it/upload_open.rs` | 1548 | P5 upload | rewrite | credential never reaches the harness (point 4); repo and folder rules are event rules |
| `cli/tests/it/upload_server.rs` | 1419 | cut: upload-server stand-in | delete | tests of the Rust stand-in of the Worker |
| `cli/tests/it/upload_server_device.rs` | 529 | cut: upload-server stand-in | delete |  |
| `cli/tests/it/upload_server_segments.rs` | 961 | cut: upload-server stand-in | delete |  |
| `cli/tests/m2_cli_lifecycle.rs` | 1944 | E2E (P1-P4 local) | rewrite | start and seal through the real daemon; drop reserve, gate and finish-budget rows |
| `cli/tests/shipped_connectors.rs` | 411 | cut: pins | delete | README P4 (pins) |
| `daemon/tests/common/capture_lifecycle_support.rs` | 579 | P1 wire capture | rewrite |  |
| `daemon/tests/common/mod.rs` | 16 | N glue | keep |  |
| `daemon/tests/common/support/control.rs` | 499 | cut: separate daemon and control API | delete |  |
| `daemon/tests/common/support/daemon.rs` | 56 | N glue | keep |  |
| `daemon/tests/common/support/mod.rs` | 142 | N glue | keep |  |
| `daemon/tests/common/support/redaction.rs` | 321 | P4 secrets | keep |  |
| `daemon/tests/common/support/status.rs` | 64 | cut: seal and signed status | delete |  |
| `daemon/tests/common/support/table_lock.rs` | 90 | cut: termination and spawn gate | delete |  |
| `daemon/tests/common/support/tree.rs` | 282 | cut: termination and spawn gate | delete |  |
| `daemon/tests/dir_modes.rs` | 338 | cut: separate daemon and control API | delete |  |
| `daemon/tests/full/capture_finish_budget.rs` | 1267 | cut: budgets and caps | delete | finish budget (H-10) |
| `daemon/tests/full/main.rs` | 12 | N glue | keep |  |
| `daemon/tests/full/process_kill.rs` | 1285 | cut: termination and spawn gate | delete | README P7 (termination) |
| `daemon/tests/health_status.rs` | 151 | cut: seal and signed status | delete | signed status (D25) |
| `daemon/tests/it/binary_surface.rs` | 32 | cut: separate daemon and control API | delete |  |
| `daemon/tests/it/capture_harness_load.rs` | 449 | cut: pins | rewrite |  |
| `daemon/tests/it/capture_ingest.rs` | 3095 | P1 wire capture | rewrite | daemon receiver; 1555-1606 and 2796-2830 pin stream and producer caps |
| `daemon/tests/it/capture_lifecycle.rs` | 3529 | P1 wire capture | rewrite | daemon capture entry; mixes seal, quota and restart rows |
| `daemon/tests/it/capture_native.rs` | 2664 | P2 subagents | rewrite | point 2; cap rows 1651-1709, 2385-2415, 2416-2479 change with the cap |
| `daemon/tests/it/capture_redaction.rs` | 4745 | P4 secrets | keep | README P1; split-at-any-byte invariance |
| `daemon/tests/it/capture_redaction_extra.rs` | 1113 | P4 secrets | rewrite | connector extra patterns; the Codex rate-limit extras go |
| `daemon/tests/it/capture_redaction_run_secrets.rs` | 379 | P4 secrets | keep |  |
| `daemon/tests/it/config_load.rs` | 160 | cut: separate daemon and control API | delete |  |
| `daemon/tests/it/control_api.rs` | 1735 | cut: separate daemon and control API | delete | README P5 |
| `daemon/tests/it/harness_contract_golden.rs` | 463 | cut: byte-for-byte goldens | delete | byte-for-byte goldens of internal facts |
| `daemon/tests/it/main.rs` | 33 | N glue | keep |  |
| `daemon/tests/it/redaction.rs` | 2629 | P4 secrets | rewrite | README P1; drop email and Codex rows (755-772, 935-968); kind list changes |
| `daemon/tests/it/registry_roundtrip.rs` | 652 | cut: separate daemon and control API | delete |  |
| `daemon/tests/it/start_race.rs` | 108 | cut: separate daemon and control API | delete |  |
| `daemon/tests/it/table_lock_group.rs` | 15 | cut: termination and spawn gate | delete |  |
| `daemon/tests/it/terminate_sim.rs` | 2714 | cut: termination and spawn gate | delete | termination escalation, simulated |
| `daemon/tests/it/workspace_layout.rs` | 643 | cut: event rules (workspace, repo, window) | delete | empty workspace outside git (event rule) |
| `platform/tests/it/artifact_root_unix.rs` | 224 | cut: platform hardening (capabilities, sleep lock, machine facts) | delete |  |
| `platform/tests/it/awake.rs` | 256 | cut: platform hardening (capabilities, sleep lock, machine facts) | delete |  |
| `platform/tests/it/disk.rs` | 137 | cut: disk full and reserve | delete |  |
| `platform/tests/it/machine.rs` | 376 | cut: platform hardening (capabilities, sleep lock, machine facts) | delete |  |
| `platform/tests/it/main.rs` | 17 | N glue | keep |  |
| `platform/tests/it/unix_snapshot_primitives.rs` | 88 | cut: platform hardening (capabilities, sleep lock, machine facts) | delete |  |
| `platform/tests/it/unix_sourcecap.rs` | 261 | cut: platform hardening (capabilities, sleep lock, machine facts) | delete |  |
| `platform/tests/it/unix_tracecap.rs` | 325 | cut: platform hardening (capabilities, sleep lock, machine facts) | delete |  |
| `protocol/tests/export_roundtrip.rs` | 1477 | Chain format (base for P5) | rewrite | chain writer; slim to what upload needs |
| `protocol/tests/it/capture_storage.rs` | 829 | Chain format (base for P5) | rewrite | quota, durability, ack storage |
| `protocol/tests/it/capture_terminals.rs` | 884 | cut: seal and signed status | delete | local finish and seal terminals |
| `protocol/tests/it/chain_golden.rs` | 102 | Chain format (base for P5) | keep | README P2; link hash; tiny, keep |
| `protocol/tests/it/connector_pins.rs` | 303 | cut: pins | delete | README P4 |
| `protocol/tests/it/connector_template.rs` | 120 | P7 harnesses | keep |  |
| `protocol/tests/it/control_consumer.rs` | 121 | cut: separate daemon and control API | delete |  |
| `protocol/tests/it/control_shapes.rs` | 689 | cut: separate daemon and control API | delete |  |
| `protocol/tests/it/harness_conformance.rs` | 379 | P7 harnesses | keep |  |
| `protocol/tests/it/harness_loader.rs` | 650 | P7 harnesses | keep |  |
| `protocol/tests/it/harness_schema.rs` | 4451 | P7 harnesses | rewrite | README P3; 114 refusal rows; keep the rows that guard points 1, 4, 7 |
| `protocol/tests/it/m2_contract.rs` | 1715 | cut: byte-for-byte goldens | delete | frozen wire bytes |
| `protocol/tests/it/main.rs` | 20 | N glue | keep |  |
| `protocol/tests/it/no_family_literals.rs` | 378 | N code rules | keep |  |
| `protocol/tests/it/pins_build_fails.rs` | 86 | cut: pins | delete |  |
| `protocol/tests/it/provenance_wire.rs` | 46 | Chain format (base for P5) | keep |  |
| `protocol/tests/it/ps_env.rs` | 24 | P4 secrets | keep |  |
| `protocol/tests/it/roundtrip.rs` | 1143 | Chain format (base for P5) | rewrite |  |
| `protocol/tests/it/signing_vectors.rs` | 54 | cut: seal and signed status | delete | Worker and Rust signing agree |
| `verifier/tests/common/capture_records.rs` | 523 | cut: seal and signed status | delete |  |
| `verifier/tests/common/decode.rs` | 58 | cut: research decoders | delete |  |
| `verifier/tests/common/mod.rs` | 81 | cut: research decoders | delete |  |
| `verifier/tests/common/records.rs` | 31 | cut: research decoders | delete |  |
| `verifier/tests/common/wire.rs` | 153 | cut: research decoders | delete |  |
| `verifier/tests/it/connector_samples.rs` | 239 | cut: research decoders | delete |  |
| `verifier/tests/it/harness_contract_golden.rs` | 487 | cut: byte-for-byte goldens | delete | byte-for-byte goldens of internal facts |
| `verifier/tests/it/m2_anthropic_decoder.rs` | 2658 | cut: research decoders | delete |  |
| `verifier/tests/it/m2_capture_integrity.rs` | 3737 | cut: seal and signed status | rewrite | README P2; keep a small reader check that fetch-run output reads |
| `verifier/tests/it/m2_chat_decoder.rs` | 2524 | cut: research decoders | delete |  |
| `verifier/tests/it/m2_decoder_v1.rs` | 1126 | cut: research decoders | delete |  |
| `verifier/tests/it/m2_hermes_capture_only.rs` | 395 | cut: research decoders | delete |  |
| `verifier/tests/it/m2_native_observations.rs` | 1737 | cut: research decoders | delete |  |
| `verifier/tests/it/m2_openai_decoder.rs` | 2149 | cut: research decoders | delete |  |
| `verifier/tests/it/m2_research_report.rs` | 2456 | cut: research decoders | delete |  |
| `verifier/tests/it/m2_run_stats.rs` | 873 | cut: research decoders | delete |  |
| `verifier/tests/it/m2_thread_decoder.rs` | 625 | cut: research decoders | delete |  |
| `verifier/tests/it/main.rs` | 27 | cut: research decoders | delete |  |
| `verifier/tests/it/v5_corpus.rs` | 718 | cut: seal and signed status | delete | README P2; frozen v5 corpus; v5 is no longer frozen |
| `verifier/tests/m2_decoder_v1_quiet_panic.rs` | 127 | cut: research decoders | delete |  |
| `scripts/test_campaign.py` | 1335 | cut: live-run tooling (matrix, campaign) | delete |  |
| `scripts/test_check_connector.py` | 411 | P7 harnesses | keep |  |
| `scripts/test_check_links.py` | 1672 | N release and repo tooling | keep |  |
| `scripts/test_check_version.py` | 128 | N release and repo tooling | keep |  |
| `scripts/test_ci_scope.py` | 134 | N release and repo tooling | keep |  |
| `scripts/test_codeowners.py` | 56 | N release and repo tooling | keep |  |
| `scripts/test_early_crosscheck.py` | 538 | cut: research decoders | delete |  |
| `scripts/test_fetch_run.py` | 695 | P5 upload | keep | server rebuild of a run (point 5) |
| `scripts/test_gate.py` | 226 | N release and repo tooling | keep |  |
| `scripts/test_install.py` | 614 | P7 harnesses | keep | install.sh (point 7) |
| `scripts/test_linux_check.py` | 107 | cut: live-run tooling (matrix, campaign) | delete |  |
| `scripts/test_linux_matrix.py` | 1219 | cut: live-run tooling (matrix, campaign) | delete |  |
| `scripts/test_matrix.py` | 1517 | cut: live-run tooling (matrix, campaign) | delete |  |
| `scripts/test_release.py` | 180 | N release and repo tooling | keep |  |
| `scripts/test_research_measure.py` | 365 | cut: research decoders | delete |  |
| `scripts/test_research_replay.py` | 325 | cut: research decoders | delete |  |
| `scripts/test_research_sample.py` | 247 | cut: research decoders | delete |  |
| `scripts/test_research_staging.py` | 426 | cut: research decoders | delete |  |
| `scripts/test_research_watch.py` | 104 | cut: research decoders | delete |  |
| `scripts/test_third_party_licenses.py` | 94 | N release and repo tooling | keep |  |
| `server/worker/test/config.test.ts` | 298 | P5 upload | rewrite |  |
| `server/worker/test/device-unit.test.ts` | 87 | cut: device flow, login, GitHub | keep |  |
| `server/worker/test/device.test.ts` | 306 | cut: device flow, login, GitHub | keep | device flow; needed while login stays |
| `server/worker/test/github.test.ts` | 184 | cut: device flow, login, GitHub | delete |  |
| `server/worker/test/helpers/client.ts` | 158 | N glue | keep |  |
| `server/worker/test/helpers/fake-r2.ts` | 57 | N glue | keep |  |
| `server/worker/test/helpers/inproc.ts` | 80 | N glue | keep |  |
| `server/worker/test/helpers/rig.ts` | 86 | N glue | keep |  |
| `server/worker/test/helpers/worker.ts` | 113 | N glue | keep |  |
| `server/worker/test/integration.test.ts` | 814 | P5 upload | keep | upload API end to end |
| `server/worker/test/lifecycle.test.ts` | 645 | cut: event rules (workspace, repo, window) | rewrite | main run replace, start window, practice, close |
| `server/worker/test/local-db.test.ts` | 590 | P5 upload | rewrite |  |
| `server/worker/test/mail.test.ts` | 108 | cut: event rules (workspace, repo, window) | rewrite |  |
| `server/worker/test/notices.test.ts` | 1064 | cut: event rules (workspace, repo, window) | rewrite | cron close, deadline commit, emails |
| `server/worker/test/quota.test.ts` | 114 | cut: budgets and caps | keep | RUN_MAX_COMPRESSED_BYTES; owner decision |
| `server/worker/test/ratelimit.test.ts` | 184 | cut: device flow, login, GitHub | keep |  |
| `server/worker/test/segments.test.ts` | 150 | P5 upload | keep |  |
| `server/worker/test/sign.test.ts` | 68 | cut: seal and signed status | delete |  |
| `server/worker/test/supabase.test.ts` | 567 | P5 upload | keep | RPC names against the website database |
| `server/worker/test/test-mode.test.ts` | 76 | cut: event rules (workspace, repo, window) | rewrite |  |
| `server/worker/test/tsconfig.json` | 26 | N glue | keep |  |
| `server/worker/test/vectors/signing.json` | 26 | cut: seal and signed status | delete |  |
