# M2 H-10 slice 4c: the capture finish budget and the spawn gate, evidence record

Date: 2026-10-02. Branch `m2/wire`. Slice 4c ships no production code and has no gate of its own; this file records what 4a and 4b measured on real binaries, the substitutions declared, and what stays open. Every figure below is preserved in this included record; the historical evidence files and reviews named in the last section are omitted from the public cut. Public design authority: [CONTRACTS.md C6](../CONTRACTS.md#c6-explicit-local-lifecycle-and-verified-report-facts), including the C6 local finish budget and capture spawn gate amendments; ruled by [DECISIONS D40](../DECISIONS.md#d40-m2-finish-keeps-the-termination-grace-the-spawn-window-is-closed-reservation-release-waits-hermes-uses-deepseek). The private implementation-plan packet that originally carried these amendments is omitted from the public cut.

## 1. What H-10 claims now

- One local finish budget `D` = 10 s (`LOCAL_CAPTURE_FINISH_BUDGET`): an 8 s drain share plus a 2 s commit reserve (`FINISH_COMMIT_RESERVE`). The launcher takes `D` when the harness exits and calls `proxy.stop()` immediately, with the proxy's shutdown deadline set to 8 s. No refusable stage begins inside the last 1 s (`FINISH_RESPONSE_MARGIN`).
- The seal request carries `remaining_finish_ms`; the daemon refuses with `finish_deadline` (stage token, HTTP 409, never `busy`) and writes no terminal. A refusal at stage 2, 4, 6, 7 or 8 sets the entry's in-memory `ExpiredFinish`; the next finish chains the pending proxy counters and one `ShutdownDeadline` gap, and binds a recorded `Confirmed` termination instead of terminating again.
- Termination is outside the budget and keeps TERMINATION.md's grace and bound: SIGTERM, grace, SIGKILL, kill sweep, reconfirm.
- CLI read timeout = `remaining + termination_allowance + CONNECT_TIMEOUT` (2 s slack), with `termination_bound(grace) = 2*grace + 2*KILL_SWEEP_TIMEOUT_MS + RECONFIRM_DELAY_MS`. The launcher's allowance is `termination_bound(0)`; standalone `goalcup seal` uses the cached manifest's grace, else `SLOW_CALL_TIMEOUT`. A timeout is an ambiguous outcome, and a later seal answers a committed finish unchanged.
- Dropped-caller latch (fix round 1, `c371675c`): `AbandonGuard` latches an attempt whose HTTP caller disconnects before stage 3, so the next finish still records the counters and a gap.
- Pre-exec spawn gate (4b): every capture child is parked before `exec` on a socketpair and released only after the daemon's durable `Registered` and its `204`. A dead launcher, a failed or ambiguous registration, or a child that dies while parked never leads to an `exec`.
- A seal that raced in while the child was parked cannot end it (the daemon's guard sees a non-harness command line and signals nothing). The CLI reads the signed status after the release and again after any `Launch` error. Only `sealed_at` present makes it end its own just-released child (`released_child.rs`, `end_released_child`: SIGTERM native-first, grace, SIGKILL). The refusal text is worded from the real outcome. The ending is printed and never chained into the seal.

## 2. Real-binary rows

All rows run the shipped `goalcup`, `goalcup-daemon` and `goalcup-verify` as debug `test-support` builds (not release), on macOS arm64. Each `row.json` records the git head, binary hashes and the observations.

| Row (test) | Commit | Command | Result |
|---|---|---|---|
| `local_finish_under_backlog_has_one_deadline` | `cfed3b09`, then `c371675c` | `cargo test --offline -p goalcup-cli --features test-support --test capture_failure_campaign` (with `GOALCUP_ROW_EVIDENCE_DIR`) | PASS. Launcher returned 16.33 s (run-02) and 16.28 s (run-03) after its finish started; ceiling 17.25 s (10 + 4.25 + 2 + 1, `capture_failure_campaign.rs:637`). Exit 1 with the "did not answer the finish" sentence; descendant alive; a fresh `goalcup seal` committed one `LocalFinish`, a second printed the identical report; termination `AlreadyStopped`. |
| `local_finish_under_stream_backlog_returns_by_the_deadline` | same | same | PASS. 271 ms (pre-implementation pin), then 158 ms and 120 ms. Exit 0, one `LocalFinish`, sealed, verifier exit 1, no upload dial. |
| `capture_cli_killed_before_registration_never_execs` | `b4a249eb` | `cargo test --offline -p goalcup-cli --features test-support --test capture_crash_campaign -- --ignored --test-threads=1 --nocapture <row>` | PASS. Child gone, workload never ran, seal `Unresolved(ProcessAuthorityUnavailable)`, `launch_missing`. |
| `capture_cli_killed_after_registration_before_release_never_execs` | `b4a249eb` | same | PASS. Seal `AlreadyStopped`, never `Confirmed`. |
| `capture_cli_killed_after_release_harness_is_owned` | `b4a249eb` | same | PASS on the `stream` workload (see substitution 2). Seal `Confirmed` on the exact instance. |
| `capture_daemon_killed_during_gate_never_execs` | `b4a249eb` | same | PASS. Launcher exits 1 after the bounded retry, no pid file, no `IngestLost`, `launch_missing`. |
| `capture_daemon_killed_after_registered_harness_is_owned_across_restart` | `b4a249eb` | same | PASS. `IngestLost` once, harness released into the new daemon's entry, seal `Confirmed`. |
| `capture_gated_child_killed_before_registration_is_refused` | `b4a249eb`, rerun `5a170e38` | same | PASS, observed differently from v3's prediction: a killed parked child is a macOS zombie with no readable creation identity, so the CLI's own `gated_handle` refuses (exit 3) before any registration; the daemon-side 400 is not reached. |
| `seal_while_parked_signals_nothing_then_cli_ends_released_child` | `b4a249eb`, rerun `5a170e38` | same | PASS. The parked child was alive and unchanged after the racing seal (exit 0, `Unresolved`); the launcher ended it 102 to 116 ms after the release (113 ms on `5a170e38`, "killed: none"); exit 1; verifier `launch_missing` and `gap_recorded`. |

Two observations in the backlog row differ from v3's prediction: the daemon did not commit by itself after the thaw (the CLI's dropped connection cancelled the handler), and the verifier read the trace as exit 0, not 1, because the frozen request carried no counters and no latch gap was written. The SIGSTOP row reads 1 when the daemon dispatched the dropped request and 0 when it never did.

### Declared substitutions

1. **SIGSTOP stand-in.** A whole-daemon SIGSTOP replaces the plan's held owner append and delayed blocking permit. Those two holds are observed only by the in-process `capture_finish_budget` rows through `cfg(any(test, feature = "test-hooks"))` seams. The real parts are the `stream` backlog, the real binaries, and a real descendant that forked from the harness and holds its stdout.
2. **`stream` workload for "harness is owned".** `plain`+`hold` ends on its first refused connection once the launcher and its proxy are killed, leaving no owned harness to seal. `stream` survives the proxy.
3. **SIGKILL-after-grace is in-process only.** No real-binary row has a TERM-ignoring harness. It is covered by `end_released_child_kills_a_term_ignoring_root_after_exactly_the_grace` and `launch_refused_on_sealed_entry_ends_released_child_within_grace` (grace 2 s, `killed: pid`) in `capture_gate`/`m2_cli_lifecycle`.
4. Weak RED on the 4b real rows: the pause points did not exist before 4b, so they could only fail waiting 60 s. The in-process `capture_registration_refused_never_execs` and the sealed-launch rows are the behavioural RED.

## 3. Measured numbers

- Owner post-stop work with a full 8 MiB encoded queue (`owner_post_stop_drain_fits_commit_reserve`, 2,097,152 source bytes): release 138.05 ms (first run) and 141.96 ms (with the 500 ms assertion); debug 3.39 s. Debug does not fit the reserve, so a debug-profile full queue is refused at `owner_drain` (clean: no terminal, sticky gap, the next `goalcup seal` completes). The release figure is the only release evidence.
- F1 (`a_blocked_evidence_queue_does_not_outlast_the_one_finish_deadline`), isolated: stage-4 owner stop 26 ms to 178 ms of about 2.0 s left (four timed runs). One full-suite run on a load-average 15 to 21 machine refused at `owner_drain`; F1 now accepts a one-shot seal or that refusal followed by one `goalcup seal`. The refusal branch is not forced by any test.
- Backlog row 16.33 s and 16.28 s against the 17.25 s ceiling; stream row 158 ms and 120 ms.

## 4. Gates

- 4a: `cfed3b09`: 2,465 passed / 0 failed / 31 ignored (build, fmt, clippy `-D warnings`, `cargo test --workspace --offline --no-fail-fast` all exit 0). Fix `c371675c`: 2,466 / 0 / 31, all four exit 0.
- 4b: `6b999eee`: 7 deterministic failures (my regression, probe before identity), fixed in `b4a249eb`. `b4a249eb`: two full runs, each 2,484 passed / 1 failed / 38 ignored, failing different known H-03 intermittents (`process_kill::a_preexisting_orphan_is_not_hidden_by_a_clean_darwin_singleton_exit`, then `finish_matrix::signed_deadline_finishes_without_cli`), each green isolated. Fix `5a170e38`: build, fmt, clippy exit 0, affected binaries only (no full workspace run on that commit).
- Merged `m2/wire` at `c20d9e92` (Hermes and H-16 merges): build 0, fmt 0, clippy 0, test 0, **2,580 passed / 0 failed / 40 ignored**. Campaigns on that tree: `capture_failure_campaign` 6/6 and `capture_crash_campaign` 12/12, 18/18.

## 5. Residuals (stated in the 4b amendment, CONTRACTS paragraph 5)

- (a) A CLI crash between the release and the status look, with a seal that raced in, leaves a running harness on a sealed entry. (a') Same outcome with the CLI alive when the status is unreadable twice. Both need two faults.
- (b) A signal killing the child between its ack and `exec` (under a microsecond); the numeric-signal race on the native child (d), plus the adoption snapshot-to-identity race.
- (c) Descendants other than the root and the re-resolved native child are never signalled (D30). (e) A descendant holding stdout still holds the tee to the finish deadline.
- (f) A late acknowledgement after the 5 s `ready_within` timeout: the child execs, `release()` returns `Err`, and the child is SIGKILLed with no grace.
- Open outside the amendment: the commit-reserve split is untested on debug builds at a full queue; F1 is load-sensitive; a TERM-ignoring process tree ends `Unresolved(ProcessAuthorityUnavailable)` on the standalone seal after the full grace (pre-existing); non-unix gate stubs were never built; the intermittents are tracked as H-03.

## 6. Reviews and evidence

Historical review and report identities (omitted from the public cut): `.superpowers/sdd/2026-10-01-m2-phase-1-local-reliability/` (local, not in git), including `task-4-design-review.md` and `task-4-design-v2-review.md` (both NEEDS_DESIGN_CHANGES, resolved in `task-4-design-v3.md`), `task-4a-review.md` (APPROVED after fix 1), `task-4b-review.md` (NEEDS_FIXES, then APPROVED conditional on the merged-branch workspace gate, which is the 2,580/0/40 run above), and reports `task-4a-report.md` and `task-4b-report.md`. Their measured facts are retained in this record; none is required or claimed present.

Historical evidence directories (local, not in git, under `target/`, omitted from the public cut): `phase1-task4a-evidence/` (logs 50-53 and 70-73 are the gates, 27 and 28 and 63 the measures, 29 and 30 F1, `rows/` the real rows, `exits.txt` explains every non-zero exit), `phase1-task4b-evidence/` (50-66 the gates, 70-76 and 87-88 the real rows, `rows/<test>/run-NN/row.json`), `merge-2026-10-02-evidence/` (`07-test.log`, `08-campaign-failure.log`, `09-campaign-crash.log`, `exits.txt`). Each directory has a `sha256.txt`; their measured facts are preserved in this record and they are not required or claimed present.
