# M2 storage and accounting fault campaign (Phase 1 Task 2 slice of T25)

**Scope: this is the Task 2 slice only. T25 is not complete.** Four serial rows on one source freeze, run
against the real `goalcup`, `goalcup-daemon`, in-process proxy and `goalcup-verify` on the synthetic
workload of `m2_synthetic_vertical`. Task 3 (crash and torn tail) and the later restart/finish rows of T25 are
not in this document. Test and evidence only: no production source changed.

Pass, Fail and Unknown below are the plan's definitions for Task 2.

- **Pass:** refusal before launch on allocation failure; an actual mid-run storage fault leaves a viable relay
  running, bounded evidence and an honest terminal or accounting outcome.
- **Fail:** host-disk fault, launch despite failed reservation, manufactured durability or exact loss, a silently
  dropped gap or uncontrolled growth.
- **Unknown:** ignored volume tests, and filesystem errors not injected at the real boundary, stay unmeasured.

## Freeze and identities

| Item | Value |
|---|---|
| Source freeze | `git rev-parse HEAD` = `dddfe6784f8db62e07d0868204198dc915ced2b7` (production source `db8bbc94`, H-09 approved). `git status` showed only the new, untracked campaign files; nothing under `crates/` was modified. |
| `goalcup` | `target/debug/goalcup`, sha256 `1a3aca0d5b90520ae00444446f044ce7923f56181bbdb2d4ed74c0bab1856095` (`cargo test --features test-support` build, the one `CARGO_BIN_EXE_goalcup` names; a plain `cargo build --offline` produced `f07ffd4f…`, replaced by this build) |
| `goalcup-daemon` | `target/debug/goalcup-daemon`, sha256 `cf30082310a2993145e42a5f244a77760201d26c2829f5b660f3b460a95d5bdd` (plain `cargo build --offline`) |
| `goalcup-verify` | `target/debug/goalcup-verify`, sha256 `5b62be910f99e4d088e417ed5a35e0db1aa6a28993dcd82c93a532b167ec866b` |
| Profile / OS / arch | `debug` (unoptimized + debuginfo), macOS 26.6.2 (Darwin 25.6.0), aarch64 |
| Filesystem | The tests' private roots are `tempfile` directories under `/var/folders/…`, on `/dev/disk3s1 on /System/Volumes/Data (apfs, local, journaled, nobrowse, protect, root data)`; every row records this in `row.json`. The host volume was never filled and no volume was mounted. |
| Test sources (after fix round 1; rows 4 and 1 rerun; rows 2 and 3 were not rerun: row 3's test body changed only in comments, row 2's not at all, and the shared support module changed code (`Alias` drop, evidence directory) that does not affect rows 2 and 3 behavior) | `crates/goalcup-cli/tests/capture_failure_campaign.rs` sha256 `bf835fbe19aeac33a8258708ce9ab507814a8eb8929ff9c2ffda049fb4437f49`; `capture_failure_support/mod.rs` sha256 `c01c2a058cae414506e858763647e4065b620621d28eaccbbfad44028a37f80a` |

Retained evidence (git-ignored, under `target/phase1-task2-evidence/`): `exits.txt` (every command, exit, log
sha256), `*.log`, `binaries-sha256.txt`, `head.txt`, and `rows/<test>/{row.json,logs/}` with each row's
observations, identities and the daemon/CLI/verifier logs. The pilot runs (`02`–`05`) and the first failing output
(`05-row4-metadata-pilot.log`, below) are kept next to the first-round rows (`10`–`13`).

**Reruns and what is retained.** Fix round 1 reran rows 4 and 1 (logs `32`, `33`). Those reruns overwrote the
first-round `rows/<test>/row.json` files for rows 1 and 4, so the current `row.json` files are the reruns' (row 2 and
3 `row.json` are first-round). The row 4 rerun `row.json` records `git_head` `74d07df9` with `git_tree_clean: false`:
the run used the fix-round changes uncommitted, which became commit `c7123b5d`. The first-round logs `10` and `13`
remain, and the figures below marked first-round (row 4 seq 94 / 547,294 B / 74,714 B) come from that earlier run,
not from the current `row.json`. The checks table lists every log.

**Row 1's `row.json` was overwritten again by Phase 1 Task 3.** Task 3 ran this control row once as its own control
(`target/phase1-task3-evidence/34-final-task2-control…log`). This support module replaced the row directory on every
run, so `rows/campaign_control_local_capture_finishes_and_verifies/row.json` now records the Task 3 run: `git_head`
`a35c93c8…` (`git_tree_clean: false`), began 2026-10-01T13:48:03Z, live durable frontier **seq 22 at 38,347 bytes**
(chain 49 lines, 77,515 bytes, verifier exit 0). The fix-round values, including the live durable frontier **seq 23
at 39,415 bytes**, survive only in log `33-fix1-campaign_control_local_capture_finishes_and_verifies.log` (sha256
`a625a1ce85e32230052b0d96b43da74501e2237956f8b0dfcfc918bf57ba5a0e`, from `exits.txt`); `row.json` no longer holds them.
The row 2 and 3 `row.json` files are first-round and row 4's is the fix-round rerun, all unchanged by Task 3. The support
module now writes each run to its own fresh `rows/<test>/run-NN/` directory (see Reproducibility notes), so a later run
cannot overwrite earlier evidence; the `row.json` files already in place from the runs above are not moved.

## What each row asserts, and what it does not

The unit and protocol suites own the byte arithmetic and are not restated: `allocation_failure_refuses_pilot`,
`reopened_reservation_is_remeasured`, `assistance_spends_the_terminal_allowance`,
`failed_sync_preserves_previous_frontier`, `local_finish_flush_failure_is_not_success`,
`a_failed_seal_write_still_leaves_the_terminal_durable`, the daemon's
`capacity_exhaustion_uses_bounded_terminal_records`, and the opt-in `reserved_chains_survive_remount_and_exhaustion`
oracle. The rows assert only what crosses the process boundary: launch or no launch, later traffic delivered
after storage loss, bounded retained records, the signed status, the CLI's outcome and the verifier's literal exit.

The faults are production boundaries, with no test hook, no daemon control route and no alternate writer:

- **Reservation:** `goalcup start --capture-reserve-bytes N`, refused by the platform allocator before the goal,
  the Launch or the harness.
- **Exhaustion:** logical. A 2 MiB reserve (1 MiB payload room, 1 MiB protected terminal allowance; only the payload room is spent) by the
  `stream` workload. No filesystem is filled.
- **Append/accounting fault:** a second hard link to the live `trace/chain.jsonl`, created outside the trace
  directory and removed only after the refusal was observed. `TraceAppendFile::append_checked` validates the
  file's link count before writing, so the refused line never reaches the file.

## Rows

All commands run serially, `CARGO_BUILD_JOBS=3`, from the worktree root:

```sh
cargo test --offline -p goalcup-cli --features test-support --test capture_failure_campaign \
  -- --ignored --test-threads=1 --nocapture --exact <test>
```

### 1. `campaign_control_local_capture_finishes_and_verifies` — PASS

First-round log `10-final-campaign_control_local_capture_finishes_and_verifies.log`, exit 0, 8.65 s of test time (11.29 s
in the first pilot, `02`). Rerun in fix round 1: log `33-fix1-campaign_control_local_capture_finishes_and_verifies.log`,
exit 0, 8.54 s, sha256 `a625a1ce85e32230052b0d96b43da74501e2237956f8b0dfcfc918bf57ba5a0e`. The observations below are
that run's, kept in its log (durable seq 23 while holding, sealed chain 77,515 bytes, 49 lines, verifier exit 0). The
current `row.json` is Task 3's later control run (durable seq 22 while holding; same 49 lines, 77,515 bytes, verifier 0),
as stated under Reruns above.

- `plain` workload with `hold`; reserve 64 MiB. While holding: run `running`, capture `recording`, delivery
  `local_only`, `quota_bytes` = `reserved_bytes` = 67,108,864 (measured physical reservation), accounting
  available, gap count 0, durable frontier present.
- `goalcup start` exit 0, "local only", no upload dial. One `LocalFinish`, last; one Launch; no Gap; every line v5.
- Final status `sealed`, gap count 0. Verifier exit 0: `integrity` `valid`, `anchor` `local_only`, `sealed` true,
  terminal `local_finish`, no findings. Chain 77,515 bytes, 49 lines.

### 2. `reservation_failure_refuses_before_goal_launch` — PASS

Log `11-final-reservation_failure_refuses_before_goal_launch.log`, exit 0, 5.62 s.

- Reserve `1 << 60`. `goalcup start` exit 1: "preflight failed: the capture chain cannot reserve
  1152921504606846976 bytes … reserve trace append stream refused: InsufficientSpace".
- No workload `pid` file, no run directory, no entry record, zero files under the artifact root, no upload dial.
  The workload never ran.

### 3. `chain_budget_exhaustion_preserves_bounded_terminal_gap` — PASS

Log `12-final-chain_budget_exhaustion_preserves_bounded_terminal_gap.log`, exit 0, 17.67 s.

Qualification of the constant (`SMALL_RESERVE` = 2 MiB), from the observed baseline: the chain through the Launch
record was **3,733 bytes** (limit asserted: 128 KiB, an eighth of the 1 MiB payload room), so about 1 MiB of room
is spent by real wire traffic, within seconds, at 16 KiB HTTP echoes and 8 KiB WS messages. The row asserts the
exhaustion was real: `used_bytes` within one 128 KiB envelope of the payload room.

- Loss first seen as `capture` `incomplete`, `gap_count` 2, accounting **available** (a refused payload is an exact,
  accounted loss), quota 2,097,152, reserved 2,097,152.
- **Relay keeps delivering:** 18 HTTP and 9 WS exchanges before the loss was first read; then, after it, at least 10
  more HTTP and 5 more WS, then 37 more HTTP and 20 more WS after a settle. Zero workload-side failures across the run
  (92 HTTP, 49 WS in total, `report.json` ok); the harness was never interrupted.
- **Sticky, bounded, not growing:** two samples 3 s+ apart with traffic between them: `capture` `incomplete` both,
  `gap_count` 3 both, `used_bytes` 1,040,955 both, chain file identical in size (1,040,955 bytes) while another
  37 HTTP / 20 WS exchanges passed. Gap records: exactly one entry-level gap and at most one per stream, all
  `StorageFull`, count equal to status and to the verifier (3), and ≤ 1 + streams opened. No per-discard-frame gap
  growth.
- **Terminal:** after release `goalcup start` exit 0, "finished locally; its terminal record is 180 and seal.json
  binds it". One `LocalFinish`, last, no record after it; the finish added no gap records; chain 1,044,429 bytes
  (≤ 2,097,152). No Assistance, Recovery or second Launch (no relaunch/replay claim).
- Verifier exit **1**: `integrity` `incomplete`, `sealed` true, `coverage_incomplete` true, finding `gap_recorded`,
  `gap_count` 3, terminal `local_finish` at seq 180. Final status `sealed`, gap history intact.

### 4. `metadata_write_failure_reports_accounting_unavailable` — PASS

First-round log `13-final-metadata_write_failure_reports_accounting_unavailable.log`, exit 0, 12.51 s. Rerun in fix
round 1 (adds the exact start-exit pin): log `32-fix1-metadata_write_failure_reports_accounting_unavailable.log`, exit
0, 12.88 s, sha256 `1714ff65336f7defb8cc610ebd68c7b003c4d7c38bf8578efc173b8eb173536a`. Reserve 16 MiB; the traffic of the
row cannot reach it. The figures below are the **current `row.json` (rerun)**; the first-round run (log `13`) saw the
same shape at other offsets: frontier seq 94 / 547,294 bytes, chain 622,008 bytes at the failure, 74,714 bytes beyond
the frontier, 108 validated records.

- Healthy before: `recording`, accounting available, gap 0, durable frontier seq 88 / 499,434 bytes, `used_bytes`
  621,199.
- Hard-link alias created (`nlink` 2). The next status showed `capture` **`failed`**, `run` `running`,
  `accounting_available` **false**; the frontier stayed seq 88 / 499,434 bytes, `used_bytes` 623,274 (the writer's last
  counted value; it moved from 621,199 only because appends landed between the healthy read and the alias). Chain file
  at the failure: 623,274 bytes, so 123,840 appended bytes sit beyond the durable frontier and are reported as not
  durable.
- Alias removed only after that observation. 4 s later, with the relay still delivering (at least 5 HTTP and 3 WS
  exchanges after the fault): the chain file is **byte-identical** (sha256 unchanged), the frontier is unchanged and
  names the chain's own seq-88 link hash and byte count, `used_bytes`, `gap_count` (0) and the failed/unavailable
  state are unchanged. There is no `Gap` record at all, and none with `lost_bytes`: accounting is unknown and was
  never converted to a loss figure, and the writer, once poisoned, was not retried after the alias vanished.
- Finish: `goalcup start` exit 0 and says "finished locally, but seal.json could not be written (no terminal record
  was written), so the trace reads incomplete". No `LocalFinish`, no `seal.json`, chain still unchanged. Final run
  `exited`, not `sealed`; `capture` `failed`; accounting unavailable.
- Verifier exit **1**: `integrity` `incomplete`, `sealed` false, `terminal` null, findings `chain_unsealed`,
  `producer_unclosed`, validated 108 records / 623,274 bytes, no torn tail.

## Checks

| Check | Exit |
|---|---|
| `cargo build --offline` (`00-build`) | 0 |
| `cargo test --offline -p goalcup-cli --features test-support --test capture_failure_campaign` (non-ignored, `01` and final `22`) | 0 (1 passed, 4 ignored) |
| `cargo fmt --all --check` (`20-fmt`) | 0 |
| `cargo clippy --offline --workspace --all-targets -- -D warnings` (`21-clippy`) | 0 |
| Rows 1–4, first-round serial runs (`10`–`13`) | 0, 0, 0, 0 |
| Fix round 1: `cargo fmt --all --check` (`30-fix1-fmt`) | 0 (log sha256 `e3b0c442…b855`, empty) |
| Fix round 1: `cargo clippy --offline --workspace --all-targets -- -D warnings` (`31-fix1-clippy`) | 0 (log sha256 `fc06a2d086be8c3a401e6faeb10df585bd10adebb20223fc7cd84f81da5dec91`) |
| Fix round 1: row 4 rerun (`32-fix1-metadata_write_failure_reports_accounting_unavailable`) | 0 (sha256 `1714ff65336f7defb8cc610ebd68c7b003c4d7c38bf8578efc173b8eb173536a`) |
| Fix round 1: row 1 rerun (`33-fix1-campaign_control_local_capture_finishes_and_verifies`) | 0 (sha256 `a625a1ce85e32230052b0d96b43da74501e2237956f8b0dfcfc918bf57ba5a0e`) |
| Fix round 1: non-ignored campaign test (`34-fix1-nonignored`) | 0 (sha256 `b5f227f3318be6335d3d5af40290543af4d8efaa9dac52cb92d0c0a772c7da02`) |

No full workspace test run: this change is test-only, and the plan's ruling is one integration gate per changed
runtime freeze.

## Observations to carry, not defects

- **First failing output, retained** (`05-row4-metadata-pilot.log`, exit 101): the pilot's own over-strict
  assertion that the finish exit be non-zero. The product's behavior is unchanged and honest: `goalcup start`
  exits per the CLI ladder (`crates/goalcup-cli/src/lib.rs`, "The exit ladder"): 0 means the command did what it was
  asked, independent of the harness's own exit (`main.rs` drops it) and of capture integrity. An unsealable finish is
  a reported outcome, not an error (`SealWriteFailed`, `seal.rs` `finish_locally`); capture failure is carried by the
  status (`Failed`, never `Sealed`) and by the verifier's exit 1 (CONTRACTS.md 2026-09-29 local-finish amendment).
  Row 4 now asserts the start exit is exactly 0 and that the output says no terminal record was written and the
  trace reads incomplete. This is a test assumption corrected, not a production defect, and the original failure is
  kept. Whether the launcher's exit should carry capture failure is a product decision for the CLI owner, not tested
  here.
- The CLI has a single wording for `SealWriteFailed`, "seal.json could not be written (no terminal record was
  written)", whether the terminal append or `seal.json` failed. In row 4 the terminal append failed and `seal.json` was
  never attempted; the row pins only "no terminal record was written" and "reads incomplete".
- Row 3 does not show the protected terminal allowance being spent: the sealed chain is 1,044,429 bytes, under the
  1 MiB payload room, so the gaps and `LocalFinish` fit in leftover payload room. Real-binary spend of the allowance
  was not observed; the protocol and daemon suites own that arithmetic.
- Row 2 uses the host APFS volume with an impossible `1 << 60` reserve, which the allocator refuses all-or-nothing
  with no bytes allocated, not an isolated size-limited volume. The isolated-volume allocation failure stays with the
  deferred opt-in oracle.
- Row 4: the verifier validated all 108 records (623,274 bytes in the rerun), including 123,840 appended bytes beyond the frozen
  durable frontier. The verifier cannot see durability; its validated prefix is not a durability claim, and the
  reported durable frontier is the daemon's status.
- Fixed settle sleeps (3 s twice in row 3, 4 s in row 4) follow bounded waits. If one were too short the row could only
  fail (for example a later gap between two samples), never pass falsely.
- Evidence rows were written under `target/phase1-task2-evidence/rows/<test>/` (no environment override), replacing that
  directory on every run. After Phase 1 Task 3 overwrote row 1's `row.json`, the support module writes each run to a new
  exclusively created `rows/<test>/run-NN/` directory (numbered after those present) and never removes anything. This
  change was checked by compile, `fmt --check`, `clippy -D warnings` and the non-ignored campaign test only; no row was rerun
  with it, so the rows above ran under the earlier replace-in-place layout.
- The status carries `capture: failed` on a writer failure, `incomplete` on exhaustion, and the two accounting
  states are distinguishable (`accounting_available`). The verifier cannot see accounting, only the chain.
- Each `row.json` records `git_tree_clean: false`: the campaign files were untracked while the rows ran. No
  tracked file under `crates/`, `connectors/` or `Cargo.*` was modified (`git status` showed only the new files).

## Deferred, and explicitly unknown

- **Literal ENOSPC** on a real full volume, and write/metadata errors from the kernel, are **UNKNOWN**. The hard
  link refuses the write at the capability's validation; it is a real production boundary, but it is not an
  `ENOSPC` return. The opt-in disposable-volume oracle (`reserved_chains_survive_remount_and_exhaustion`,
  `GOALCUP_T03R_VOLUME=1`) was not run in this slice. No volume was mounted and the host disk was not filled.
- Power loss and OS/device crash semantics; `kill -9` does not prove them (Task 3).
- Other filesystems, other OS versions and architectures: one APFS, one macOS 26.6.2 arm64, `debug` profile.
- RSS and memory slope under exhaustion and under the fault: not measured.
- Finer frontier timing: the rows assert the frontier's identity and that it did not move, not sync latency
  distribution or exactly when between appends the failure was latched.
- The sync-boundary diagnostic (hard link observed first at a periodic sync with no append pending) is a separate
  case from the append refusal; in these runs continuous traffic made the append boundary the one observed.
- Not T25 complete: crash, torn tail, restart/seal-only behavior and the remaining T25 rows are Task 3 and later.
