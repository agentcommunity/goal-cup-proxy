# M2 recorder crash and torn-tail campaign (Phase 1 Task 3 slice of T25)

**Scope: this is the Task 3 slice only. T25 is not complete.** Four serial rows on one source freeze, run against
the real `goalcup`, `goalcup-daemon`, in-process proxy and `goalcup-verify` on the synthetic workload of
`m2_synthetic_vertical`. The storage and accounting slice is `m2-failure-campaign.md` (Task 2). Test and evidence
only: no production source changed, and no crash repair, readmission or resume was implemented.

Pass, Fail and Unknown below are the plan's definitions for Task 3.

- **Pass:** exact prefix and torn bytes retained; a viable relay's continuation measured independently; fresh-epoch
  recovery not confused with daemon restart or agent resume.
- **Fail:** any automatic repair, readmission, agent restart, lost-evidence erasure, or a nonzero exit normalised
  into a pass.
- **Unknown:** OS/device power loss, crashed-relay survival and arbitrary v5 readmission stay unsupported and
  unmeasured.

## Freeze and identities

| Item | Value |
|---|---|
| Source freeze | HEAD at task start `c7123b5d1da83e7ebefedae34dc94b72e11072c2`; the runs below were made at `a35c93c8cbf6cb1a503fcf908776f2a44c7c75fb` (a docs-only commit on `docs/qualification/m2-failure-campaign.md`; `git diff c7123b5d a35c93c8 -- crates Cargo.toml Cargo.lock connectors` is empty). The campaign files were untracked or uncommitted while the rows ran, so every `row.json` records `git_tree_clean: false`; they became the commit that adds this document. Nothing under `crates/*/src` was modified. |
| `goalcup` | `target/debug/goalcup`, sha256 `1a3aca0d5b90520ae00444446f044ce7923f56181bbdb2d4ed74c0bab1856095` (the `--features test-support` build the tests' `CARGO_BIN_EXE_goalcup` names; identical to Task 2's) |
| `goalcup-daemon` | `target/debug/goalcup-daemon`, sha256 `cf30082310a2993145e42a5f244a77760201d26c2829f5b660f3b460a95d5bdd` (plain `cargo build --offline`; identical to Task 2's) |
| `goalcup-verify` | `target/debug/goalcup-verify`, sha256 `5b62be910f99e4d088e417ed5a35e0db1aa6a28993dcd82c93a532b167ec866b` (identical to Task 2's) |
| Profile / OS / arch | `debug`, macOS 26.6.2 (Darwin 25.6.0), aarch64 |
| Filesystem | The tests' private roots are `tempfile` directories under `/var/folders/…` on `/dev/disk3s1 on /System/Volumes/Data (apfs, local, journaled, nobrowse, protect, root data)`. The host volume was never filled and no volume was mounted. |
| Test sources | `crates/goalcup-cli/tests/capture_crash_campaign.rs` sha256 `4e5e9e8af3e6b89f9652bf1d35e928a242a81f1eb236fd807ea701d7b1021d9f`; `capture_crash_support/mod.rs` sha256 `9d455e056b1e1c3fc794de27429cc0d9daa7b76a94fb138251c933477e9a9cf6`; shared `m2_vertical_support/mod.rs` (one additive read accessor, below) sha256 `2f2e7305504ffeffaa3654a71b8ea3701f8ea939e9076651b5ec82f628370a98`. The four final rows ran on exactly these sources. |
| Fix round 1 sources | After the four final rows, fix round 1 changed comments and two assertion-message strings in `capture_crash_campaign.rs` (now sha256 `272755f3b735e12b3043e37f584bd6720b44686d23d40e3981e3d661393c2098`; no assertion condition changed, verified from the diff) and made Task 2's `capture_failure_support/mod.rs` write numbered `run-NN` directories (sha256 `44b4d08e9d64739336130aae974d38f0fc56b7b16a0512c4474f138213c3811a`). The four rows were **not** rerun: the hashes in the row above are the sources the rows ran on. `capture_crash_support/mod.rs` and `m2_vertical_support/mod.rs` are unchanged. |

Retained evidence (git-ignored, under `target/phase1-task3-evidence/`): `exits.txt` (every command, exit, log
sha256), numbered `*.log`, `binaries-sha256.txt`, `sources-sha256.txt`, `head.txt`, and
`rows/<test>/run-NN/{row.json,logs/,chain-*.jsonl,...}`. Each run takes the next unused `run-NN` directory, created
exclusively; nothing is overwritten or removed, so the pilot and failed runs stay next to the final ones.

## What each row asserts, and what it does not

The daemon, protocol and sender suites own the protocol semantics and are not restated: `capture_lifecycle.rs`
(`a_restart_never_rebuilds_a_capture_row_as_a_legacy_runtime`, `a_registered_run_reopened_before_its_launch_chains_the_loss`,
`a_restart_keeps_the_gap_history_through_the_seal`, `torn_tail_cannot_resume`, `recorder_recovery_preserves_gap_without_assistance`),
`m2_synthetic_vertical.rs::production_recovery_keeps_relay_and_gap_history`, and the `m2_capture_sender` / `m2_emitter` /
`m2_ipc_compose` recovery suites. The rows assert only what crosses a process boundary: the bytes on disk before and
after each inspection, the relay's delivery measured by the workload and by the upstream endpoint, the signed
status, the control API's refusals, the CLI's outcomes and the verifier's literal exit.

### The faults, and why each is a real boundary

- **Crash.** `kill -9` of the one daemon the test spawned (`Vertical::daemon_pid`), under live `stream` traffic. The
  signal helper refuses any pid whose command line does not name this test's private root, so the stray daemons of
  earlier work under `/private/tmp` cannot be reached from the tests.
- **Restart under the same private registry.** A second real `goalcup-daemon --foreground`, started by the test from
  the first daemon's own configuration file, so it opens the same redb registry, artifact root and trace directory.
- **Torn tail: a real partial append, through an operating-system limit, not a hook.** No env- or control-armable fault
  knob exists in the shipped or `test-support` binaries: the `ChainWriter` failure hooks are in-process `test-hooks`
  only. The row starts the daemon child under `RLIMIT_FSIZE` (6 MiB, hard and soft) with `SIGXFSZ` ignored, set in
  the child only through `pre_exec` in test code. The production writer's own `write` is then cut short by the kernel
  mid-line (a short write of exactly the bytes up to the limit, probed first on this host) and its next `write` fails
  `EFBIG`; `TraceExport::append` already documents "some prefix of the line may be on disk" and records
  `accounting_available = false`. The test writes no byte of the chain. **Classification:** this is a process resource
  limit at the production capability boundary, in the same family as Task 2's accepted hard-link fault; it is not a
  compiled-in knob. The coordinator's review accepted it as a bounded existing seam. **The fault shape is a kernel short
  write plus `EFBIG` with the daemon still alive, followed by a separate `kill -9`; it is not a crash inside a write.**
  The 6 MiB figure sits above every other file the daemon keeps (the registry is about 2.6 MiB; a 3 MiB first pilot
  failed in the registry before any entry existed, see Retained failures).
- **Receiver-alive fresh epoch.** The T23 seam: the owned daemon is `SIGSTOP`ped for 25 s and `SIGCONT`ed. The daemon's
  pid and start time are asserted unchanged. The sender's bounded queue fills, cuts the epoch without a `Close`, and a
  fresh producer is admitted at the next safe boundary. This is a sender-cut-by-stall, **not** a peer-initiated TCP
  close: no real-binary seam closes one ingest connection while the receiver stays up (the ingest address and secret are
  internal to `goalcup start`).
- **Independent measurements.** The workload's own byte-checked exchanges (`events.jsonl`), and the controlled upstream
  endpoint's counters, read through one added accessor `Vertical::endpoint_counters()` in the shared
  `m2_vertical_support` (additive, `#[allow(dead_code)]`, no behaviour change; T23 and Task 2 rows are unaffected).
  `echo_requests` and `ws_messages_in` must equal the workload's sent counts exactly, which excludes a replayed request.

## Rows

All commands run serially, `CARGO_BUILD_JOBS=3`, from the worktree root:

```sh
cargo test --offline -p goalcup-cli --features test-support --test capture_crash_campaign \
  -- --ignored --test-threads=1 --nocapture --exact <test>
```

Final logs and sha256 are in the Checks table. Figures below are from the final run of each row
(`rows/<test>/run-04`, `run-05`, `run-03`, `run-03` respectively).

### 1. `daemon_exit_preserves_prefix_and_honest_recovery_limit` — PASS

Log `30`, exit 0, 29.78 s of test time.

- Healthy before the crash: recording, gap 0, durable frontier seq 85.
- `kill -9` of the owned daemon. The chain it left (`chain-after-kill.jsonl`) is 107 complete lines, 621,200 bytes,
  sha256 `e225311a…`, ending on a newline and a pure extension of the pre-kill read. The launcher, harness and proxy were the same processes
  (pid and start time unchanged). The workload completed 10 echoes and 5 WebSocket messages that began after the
  kill with zero failures, and the chain did not change while the daemon was dead.
- Verifier on the dead trace, with every file's sha256 unchanged around it: **exit 1**, `integrity: incomplete`,
  `sealed: false`, `torn_tail: false`, findings `chain_unsealed`, `producer_unclosed`, validated 107 records / 621,200 bytes.
- Restart over the same registry. The reopened status is `running` / `incomplete`, `gap_count` 1, accounting available,
  durable frontier seq 107 at 621,712 bytes (`assistance_observed: false`). The restart added **exactly one record**,
  512 bytes: a stream-less `Gap` with reason `ingest_lost` and no byte count. The dead daemon's bytes are a prefix
  of the reopened chain. Resume and resume-preview, through the control API the CLI uses, both answer 409
  "a capture entry cannot be resumed: seal it instead".
- No movement: across a 20 s window (longer than the sender's 1+2+5+10 s reconnect schedule) with the relay still
  delivering, the chain hash, the set of producers and the status did not move. **This observes no movement; it is not
  a process-level proof that the daemon refuses readmission.** The restarted daemon binds a new ingest listener on an
  OS-chosen port (`goalcup-daemon/src/runtime/daemon.rs:396-404`) and the surviving `goalcup start` keeps the first
  daemon's ingest address for its whole life (`goalcup-cli/src/commands/start.rs:1679-1685`), so the old producer could
  not reach the new daemon whatever its policy. The refusal itself is proven only by the in-process
  `capture_lifecycle.rs` restart tests (`a_restart_never_rebuilds_a_capture_row_as_a_legacy_runtime`,
  `a_registered_run_reopened_before_its_launch_chains_the_loss`, `a_restart_keeps_the_gap_history_through_the_seal`).
  Resume and resume-preview refusal, by contrast, is a real control-API answer.
- Finish. `goalcup start` exits **1** with "the daemon is not reachable ... seal": it captured the first daemon's
  control port, which died with it (CLI ladder rung 1, `lib.rs`). A fresh `goalcup seal` exits 0 and writes the terminal.
  The sealed chain is the restart's chain plus one `LocalFinish` (109 records, 622,361 bytes), one Launch, zero
  Assistance, zero Recovery, one run directory, the same harness pid.
- Honest limit: 195 echoes were delivered after the crash and none appears in the evidence; the loss is one unknown
  `IngestLost` interval, never reconstructed. The upstream saw 205 echo requests and 107 WebSocket messages, exactly the
  workload's counts.
- Final verifier: **exit 1**, `incomplete`, `sealed: true`, `coverage_incomplete`, `gap_count` 1, terminal `local_finish`, findings `gap_recorded`, `producer_unclosed`.

### 2. `torn_tail_is_preserved_and_nonresumable` — PASS

Log `31`, exit 0, 43.73 s.

- The daemon child ran under `RLIMIT_FSIZE` 6,291,456. The production writer failed: status `capture: failed`,
  `accounting_available: false`, `used_bytes` 6,286,608 (the complete lines only), durable frontier seq 917 at
  5,942,178 bytes.
- The chain file is **exactly 6,291,456 bytes** with 971 complete lines and a 4,848-byte tail with no newline that
  begins `{"protocol_version":5,` and does not parse as an envelope: a real mid-line cut. Retained as
  `chain-torn.jsonl` (sha256 `3fce3fba…`) and `torn-tail.bin`.
- The relay lived on: 10 echoes and 6 WebSocket messages that began after the failure, zero failures. Then `kill -9`
  of that daemon.
- Verifier on the dead trace, every byte unchanged around it, run twice with identical output: **exit 2**,
  `integrity: invalid`, `torn_tail: true`, `sealed: false`, `terminal: null`, findings `torn_tail`, `producer_unclosed`,
  validated 971 records / 6,286,608 bytes (the validated prefix).
- Restart over the same registry. The torn chain is not reopenable, so the daemon holds no capture entry for it.
  Each attempt left every byte of the trace unchanged (whole-tree sha256 before and after): `goalcup status` exits **1**
  "no status for that entry"; resume and resume-preview answer 409 "cannot be resumed"; `goalcup seal` exits **3**
  ("malformed control response: a v5 capture entry is sealed with a capture seal request") and writes no `seal.json`.
- Across the 20 s window and through the finish: the trace's whole-tree sha256 never differed from the crash's, the
  relay kept delivering, the launcher exited **1** "daemon is not reachable", and the final chain is byte-identical to
  `chain-torn.jsonl`, the tail included. No truncation, no replay, no seal, no gap record (appending one would first
  require repairing the tail), zero Assistance, zero Recovery, one Launch. The upstream saw 329 echo requests and 172
  WebSocket messages, exactly the workload's counts. Final verifier again **exit 2**.

### 3. `disconnect_reconnects_fresh_epoch_without_replay` — PASS

Log `32`, exit 0, 43.99 s.

- Receiver stalled 25 s and continued; same daemon pid and start time before and after. 332 exchanges completed during
  the stall, none failed. Status after recovery: `incomplete`, `gap_count` 3, `assistance_observed: false`. Two fresh
  producers recorded by `Recovery`; wire streams resume under them.
- The finish succeeded through the same daemon (`goalcup start` exit 0). The final chain is byte-for-byte an extension of
  the chain retained just after recovery (`chain-after-recovery.jsonl`): the gaps are permanent. No request payload is
  retained twice (no replay into the chain). The upstream saw 311 echo requests and 168 WebSocket messages, exactly the
  workload's counts (no provider retransmission). Harness pid and command line unchanged, one Launch, one run
  directory, zero Assistance, one `LocalFinish`.
- Final verifier **exit 1**, `incomplete`, `sealed: true`, findings `gap_recorded`, `producer_recovered`,
  `recovery_names_unrecorded_producer`; its `gap_count` equals the status's (3).

### 4. `capture_recovery_emits_no_assistance` — PASS

Log `33`, exit 0, 67.42 s. Both recovery shapes in one run: a receiver-alive fresh epoch (stall), then the daemon
killed and a new one over the same registry.

- After the first recovery: a `Recovery` record exists, `assistance_observed: false`, zero Assistance records.
- After the crash and restart: the first recovery's records survive as a prefix (823 records, 5,275,851 bytes at the kill);
  the restart added one stream-less `ingest_lost` gap (gap count 3 to 4); Recovery count unchanged; zero Assistance;
  resume and preview refused; no chain, producer or status movement across the 20 s window (observed only, see row 1);
  the harness pid, start time and command line
  unchanged before the finish.
- Finish as in row 1 (launcher exit 1, then `goalcup seal` exit 0). Final chain: one Launch, zero Assistance, one
  `LocalFinish`, one run directory, same harness pid, no payload retained twice; upstream 525 echo requests and 281
  WebSocket messages, exactly the workload's. Status `sealed`, `assistance_observed: false`. Final verifier **exit 1**,
  `incomplete`, `sealed: true`; no finding mentions assistance.

### Control

Task 2's `campaign_control_local_capture_finishes_and_verifies` was run once on the same binaries: log `34`, exit 0
(verifier **exit 0**, `valid`, sealed `LocalOnly`, zero gaps). Running it **overwrote** Task 2's
`target/phase1-task2-evidence/rows/campaign_control_local_capture_finishes_and_verifies/row.json` (that support module
replaced the row directory on every run). The file now records this Task 3 control run: `git_head` `a35c93c8…`, began
2026-10-01T13:48:03Z, live durable frontier seq 22 at 38,347 bytes, chain 49 lines / 77,515 bytes, verifier 0. Task 2's
fix-round values (durable seq 23 at 39,415 bytes, and the rest) survive only in
`target/phase1-task2-evidence/33-fix1-campaign_control_local_capture_finishes_and_verifies.log` (sha256
`a625a1ce85e32230052b0d96b43da74501e2237956f8b0dfcfc918bf57ba5a0e`). `m2-failure-campaign.md` states this, and its
support module now writes numbered `run-NN` directories so it cannot recur.

## Exit table

| Boundary | Observed | Expected by the plan |
|---|---|---|
| Honest sealed incomplete chain (rows 1, 3, 4) | verifier 1 | 1 |
| Unsealed prefix after a clean-boundary kill, before restart (rows 1, 4) | verifier 1 (`chain_unsealed`) | not named by the plan; pinned as observed |
| Torn invalid bytes (row 2, before restart and at the end) | verifier 2 | 2 |
| Zero-gap no-fault control (Task 2 control row) | verifier 0 | 0 |

No other verifier exit appeared, so no oracle was normalised.

## Findings and observations (not production changes)

1. **A launcher that outlives its daemon cannot seal against the restarted one: a stale client.** `goalcup start`
   builds its control client once from `control.json` (`goalcup-cli/src/main.rs:344`), which holds the port **and** the
   control token, and reuses that client at its finish (`commands/start.rs:1841`, `finish_locally`). After a restart the
   old port is dead, so the finish fails "the daemon is not reachable" and the launcher exits 1 (ladder rung 1) while a
   daemon is in fact up. A fixed control port would not fix it: the token is minted afresh on every daemon start
   (`goalcup-daemon/src/runtime/daemon.rs:594`), so the stale client would fail as unauthorized instead. The restart
   contract is status and seal only, and a fresh `goalcup seal` seals. Routed as a product finding (CLI/H-10 owner, a new
   ledger item separate from H-10's spawn-registration and drain-budget scope); nothing is sealed falsely and the data is
   never at risk.
2. **`goalcup seal` on a torn, non-reopenable entry exits 3 with a misleading message** ("malformed control response ...
   sealed with a capture seal request"): the CLI chose the legacy seal route because the daemon has no status for the
   entry. The outcome is correct (refused, nothing written); the wording and rung are imprecise. The row pins the exit
   code 3, not the wording; a later fix to a clean refusal will flag it here. Route to the CLI owner.
3. **A torn chain cannot record the restart's `IngestLost` interval**, because reopening validates every line and refuses a
   missing newline; recording it would need a repair that the contract forbids. The interval stays unrecorded and the
   verifier reports the torn tail and the validated prefix. This is the honest limit, not a defect.
4. **Durable frontier and verifier prefix differ by design.** Row 2's validated prefix is 971 records while the daemon's
   durable frontier was seq 917: 53 appended-but-not-synced records (seq 918 to 970). The verifier cannot see durability.
5. A client closing an idle keep-alive echo connection is counted by the endpoint as `interrupted_requests` (always 1
   per run); it says nothing about replay and is recorded, not asserted.
6. The "kill landed between appends" precondition of rows 1 and 4 (`chain-after-kill` ends on a newline) is asserted, and
   held in every run. A SIGKILL timed inside a single `write` would fail that assertion with a message pointing at
   row 2; it has not happened, and a one-off failure there is a flake in the precondition, not a defect.

## Deferred / UNKNOWN

- **OS or device power loss.** SIGKILL does not prove power-loss safety. UNKNOWN.
- **Crashed relay survival.** Only the daemon is killed; the relay (`goalcup start`) surviving is measured, its own crash
  is not. UNKNOWN.
- **Arbitrary v5 readmission.** At process level the rows show only that nothing was appended and no producer, status or
  chain movement occurred after a restart (the surviving sender cannot reach the restarted daemon's new ingest port, see
  row 1), and that resume and preview are refused by the control API. The daemon's refusal to admit a producer after a
  restart is proven only by the in-process `capture_lifecycle.rs` tests; admitting a producer is not implemented or
  measured. UNKNOWN at process level.
- **Peer-initiated TCP disconnect with a live receiver.** No real-binary seam exists; covered only at the unit/IPC level
  by `m2_capture_sender.rs` and `m2_ipc_compose.rs`. UNKNOWN at process level.
- **Other filesystems and operating systems**, a kill timed inside an append (non-deterministic, not attempted), a
  daemon crash during seal or finish, and RSS. The torn row is **not** a crash inside a `write`: the kernel cuts one
  `write` short and fails the next with `EFBIG` while the daemon stays alive (capture `failed`, records beyond the durable
  frontier), and a separate `kill -9` follows. It covers the on-disk torn shape and every restart and inspection
  behaviour that depends on it, not the timing of a crash inside an append.
- **Guard deferred:** the torn row does not assert that no file other than the chain reached the 6 MiB limit (the registry,
  about 2.6 MiB, also lives under it). It holds today (the chain length equals the limit and status kept being served); a
  guard would need a row 2 rerun to validate, so it is not added in fix round 1.
- The `RLIMIT_FSIZE` seam and the `endpoint_counters()` accessor were accepted by the coordinator's review.

## Checks

| Log | What | Exit | sha256 |
|---|---|---|---|
| `00-build` | `cargo build --offline` | 0 | see `exits.txt` |
| `06-compile` | campaign test compile | 0 | see `exits.txt` |
| `22-clippy` | `cargo clippy --offline --workspace --all-targets -- -D warnings` | 0 | `e4a6ffca09675561a03affb623232d9e541a81375d4c7300d7438d90162cc676` |
| `23-fmt-final` | `cargo fmt --all --check` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (empty output) |
| `30-final-daemon_exit_preserves_prefix_and_honest_recovery_limit` | row 1 | 0 | `bb65a7f3c501df6d6000cd2b185dd85ca425ea71c63a7a58271a309aca06bb13` |
| `31-final-torn_tail_is_preserved_and_nonresumable` | row 2 | 0 | `d6ef5440559e4b2be9991abbaff7120bb45202741178b771b6f7bed548021a27` |
| `32-final-disconnect_reconnects_fresh_epoch_without_replay` | row 3 | 0 | `b533d54d917ce318630c1b419b8d991683fa7cdc3f0a2c3324a6aa6ad9d4a3fd` |
| `33-final-capture_recovery_emits_no_assistance` | row 4 | 0 | `178a27dcb5dbfceb53e9d10e10eee08b24a34cf4db1169b1270d6d84f16bd2b9` |
| `34-final-task2-control` | Task 2 control row | 0 | `dbd16b28324d3bb734d3962e832530c38028c010a277b83135430280676f59e2` |
| `35-final-nonignored` | campaign file, non-ignored run (4 ignored, 0 failed) | 0 | `d12f53ddc92460dea7f37cfaac6288f88f5353a9b9a4c0b3e29810772fc6feeb` |

Fix round 1 checks (compile and lint only; no row rerun): `40-fix1-fmt` exit 0 (`e3b0c442…`, empty);
`41-fix1-clippy` (`cargo clippy --offline --workspace --all-targets -- -D warnings`) exit 0, sha256
`7720fda8c7d0b66c261d63f1bb039f05b5929c5411cd325071899957422bdbc8`; `42-fix1-nonignored-failure-campaign` (Task 2 campaign
non-ignored, `--features test-support`) exit 0, sha256 `0aae14ec0819c96a42e054167719f166aa3b810d93ba22cd85695f07dee5355c`;
`43-fix1-nonignored-crash-campaign` exit 0, sha256 `385f3ac6c45d52b45f699c562cdaa842c4b4505522bc7f9b5524103b3949c057`.

No workspace test run: the change is test-only. The ignored rows were run by name, serially, `--exact`.

## Retained failures and pilots

- `01-pilot-row1.log`, `01b-pilot-row1.log`: first compile error (private trait path) and the exploratory row 1 run
  that fixed the oracles above.
- `02-pilot-row2.log`: with a 3 MiB limit the daemon's own registry (about 2.6 MiB) could not grow and entry creation
  failed ("File too large", `goalcup: conflict: capture setup failed`) before any chain existed. This is what sized the
  limit; it is a property of the limit, not of the recorder. `03-pilot-row2.log` (24 MiB, the chain did not reach it in the
  60 s wait) and `04-pilot-row2.log` (6 MiB, the successful pilot) followed.
- `10`, `11` row 1 first runs (`10` failed on a too-strict `interrupted_requests` assertion, since replaced);
  `12`, `13`, `14`, `15`, `16`, `17`, `18`: first rows 2-4 runs, including two compile errors and two assertion
  mistakes of mine (a harness identity check after the harness had exited; row 4's gap count ignoring gaps already chained
  by the first recovery). All are corrections to the tests; none is a recorder defect.
- Every earlier `rows/<test>/run-NN/` directory remains next to the final one.

No process left running: after every run `pgrep -fl goalcup-daemon` shows only the three pre-existing H-13 strays
(pids 52294, 69232, 95284), which no test signalled.
