# The local production vertical on a synthetic workload

> **Current evidence (2026-10-02).** T23 vertical and the G-2 Part B run on `3c542aa5` (last section). Ticket state: [STATUS](../../STATUS.md), [LEDGER](../../LEDGER.md); index: [README](README.md).

> The installed-path capture lifecycle end to end on this Mac with no service: the shipped
> `goalcup-daemon`, `goalcup start --goal-file` (in-process proxy, IPC sender), the reserved chain,
> native capture, the local finish, `goalcup status`, `goalcup seal`, `goalcup-verify`. Only the
> workload and its TLS upstream are fixtures. Not real harness support, a signed anchor, delivery
> or a long unattended claim. The config requires a loopback upload URL and placeholder key.
> Tests trap upload dials; Part B checks local-only verification and no receipt.
## 1. The fixtures
- **Workload:** `crates/goalcup-cli/tests/fixtures/m2-controlled-workload.py` (`/usr/bin/python3`,
  stdlib, E02's `early-load.py` helpers loaded by path). It answers `--version`, reads the goal
  once from its last argument, writes native JSONL under `$HOME/native/`, talks CONNECT+TLS HTTP,
  SSE and WebSocket to the controlled endpoint through `HTTPS_PROXY`, writes `result.txt` and a
  size-and-sha256 report outside the recorder. Modes: `plain`, `sparse`, `partial-native`,
  `split-secret`, `stream`, `load`.
- **Connector:** `tests/fixtures/m2-controlled-workload/manifest.toml`, a development connector no
  build pins (admitted through `--dev` only). Its capture executable is `/usr/bin/python3`, pinned
  by sha256 and version when a test or the driver writes the copy both binaries read
  (`GOALCUP_CONNECTORS_DIR`, `connectors_dir`); `home = "empty"`, `ca_env = ["NODE_EXTRA_CA_CERTS"]`,
  one native store `{home}/native/*.jsonl`, host `api.e00.localhost` (`/v1/{echo,sse,ws,bulk}`,
  hint `e00-controlled`), tunnel `auth.e00.localhost`. Family and hint are unknown to every
  decoder: bytes are kept, semantics stay unavailable.
- **Upstream:** the E00 controlled endpoint (`examples/m2_capture_support/{endpoint,wire}.rs`), in
  the test process or alone as `m2_capture_experiment endpoint OUT_DIR TEST_ID`. `goalcup start`
  trusts its CA through `GOALCUP_TEST_UPSTREAM_ROOTS`, which only a `test-support` build reads.
## 2. The eight tests (`crates/goalcup-cli/tests/m2_synthetic_vertical.rs`)
```sh
CARGO_BUILD_JOBS=3 cargo build --offline
CARGO_BUILD_JOBS=3 cargo test -p goalcup-cli --offline --features test-support --test m2_synthetic_vertical -- --ignored --test-threads=1
```
Each runs `goalcup start --harness m2-controlled-workload --dev --goal-file GOAL --capture-reserve-bytes
67108864` in its own process group, then the `goalcup status ENTRY --json`, `goalcup seal ENTRY` and
`goalcup-verify --capture-integrity TRACE` binaries.
| Test | What it holds against what |
|---|---|
| `local_finish_without_service_is_complete` | status read during the SSE response: `Running`, `Recording`, `LocalOnly`, a durable frontier. Every sent body and WS message retained byte for byte (sha256); one Goal stream equal to the goal, named by the one Launch; native bytes equal to the file, hint `None`; wire hint `e00-controlled`. Verify exit 0, `valid`, `local_only`, no findings, `LocalFinish`, `AlreadyStopped`, no receipt or ACK; research summary `analysis: unavailable`. A second seal answers the same terminal, chain sha256 unchanged, 0 connections to `upload_url` |
| `reserved_chain_precedes_production_launch` | when the workload writes its pid (its first act) the chain file has `st_blocks × 512 ≥` the 64 MiB requested while its length is under 1 MiB; status `quota_bytes = reserved_bytes =` requested. A 2^60 request: exit 1 naming the reservation, the workload never ran, no run directory, entry record or entry artifacts |
| `sparse_stream_has_incremental_durable_evidence` | one SSE response of 10 events 10 s apart (~100 s), `goalcup status --json` every 0.5 s: at least 10 segments spread over > 80 s, each durable before the stream ended; capture-to-append < 2 s and append-to-durable < 5 s asserted as guards, measured values printed against the 250 ms and 1 s targets |
| `boundary_secret_is_absent_from_chain_and_logs` | the E02 sentinel split across two request chunks, a 4 096-byte WS frame boundary both ways, two native writes 300 ms apart, and a request cut off inside the secret. No `sk-ant-` byte or proxy credential in the chain, in any file under the whole test root (logs, config, home, workspaces, connectors, trace and reports) (the native source aside); each carrier has an `anthropic_key` span; the cut request keeps every byte sent, replaced not released, `End Interrupted`; verify exit 0 |
| `native_partial_line_survives_local_finish` | a native file ending 500 bytes into a line: the stream retains it byte for byte, `End Complete`, no gap; verify exit 0 |
| `clean_producer_close_and_unexpected_eof_differ` | a launcher that ends with its harness: one `ProducerClosed` for the proxy's producer after its last record, `final_seq` covering every framed record, no gap, exit 0. `kill -9` of the `goalcup start` pid mid-stream: the harness lives, its next exchanges fail (proxy gone), status still `Running`; `goalcup seal` ends it `Confirmed`; no `ProducerClosed`, an `IngestLost` gap for the producer, exit 1 `incomplete` |
| `production_recovery_keeps_relay_and_gap_history` | `kill -STOP` of the daemon for 25 s under stream traffic (16 KiB echoes every 100 ms, 8 KiB WS messages every 200 ms): every exchange passed, none took 5 s; the first `QueueFull` Recovery names the first wire producer or an epoch with no retained records; a connection it opened is re-observed `already_open` in a fresh epoch; capture resumes; no Assistance; status `Incomplete` with the gap kept after the seal; verify exit 1, `producer_recovered`. The loss is printed, not bounded |
| `finish_drains_once_and_never_appends_after_terminal` | `goalcup seal` while an SSE response streams: `Confirmed` within 25 s; the chain is byte-identical after the launcher's own finish and a second seal; one `LocalFinish`, the last line; every line v5; `seal.json`; the open response keeps its prefix with a recorded tail; exit 1 `incomplete` |
## 3. E02 Part B on the shipped path: method
`spikes/m2-first-release/t23/run.sh load-smoke` (180 s, plumbing) first, then `run.sh load`, once,
on the tree recorded in `identity.txt`, in a quiet window (at least 40 minutes, no `cargo`, `rustc`, `hdiutil`, `cc`
or `ld`: the driver refuses; every lane and agent paused; load average logged at start).
- **Build:** release, from the recorded tree. `goalcup` is built with `--features test-support`
  which adds only `GOALCUP_TEST_UPSTREAM_ROOTS` on the command paths driven here;
  `goalcup-daemon` and `goalcup-verify` are the plain release builds.
- **Run:** the dev connector in `load` mode runs `early-load.py`'s `full` schedule (plan sha256
  `c6848013…57adcf34`, 125 706 240 B logical) through the in-process proxy, with
  `--capture-reserve-bytes 536870912`. The proxy runs on one worker thread; E02's runner ran four.
- **Stall:** `kill -STOP` of the daemon at load + 720 s, `kill -CONT` 40 s later; sampler and
  `goalcup status --json` each second. No shipped seam reports the stall's release, so the
  driver's `summary.json` holds the endpoint's counters and its own stall record, whose
  `released_by` restates the chain: `queue_full` if a `QueueFull` Recovery came inside the stall.
  The stall is fixed at 40 s; no `queue_full` Recovery stamped inside it means **FAILED**.
  E02 held until QueueFull (15.5 s, 12 288 B lost); this stall is timer-ended. Held time,
  Recovery count and loss are reported without comparing them; the six checks and verify result are comparable.
- **Pass (asserted):** the corrected cross-check (`early-crosscheck.py`, `MATCH_TOL_S = 0.05`,
  sha256 `4f6045d6…`) ok on all six checks, and `goalcup-verify --capture-integrity` exit 1,
  `incomplete`, sealed, no torn tail, `producer_recovered`. The chain must show a first Recovery
  naming the first wire producer or an unrecorded epoch, an `already_open` Open under a fresh
  Recovery producer, and no `ProducerClosed` for the lost producer. The frozen v1 (`7689ea5`,
  `4d9dd428…`, 1.0 s) runs beside it and is reported.
- **Reported against targets, not asserted** (E02 values: E01 binary, in-process sink): wire
  safe-byte latency outside the stall ≤ 250 ms [237.2 ms]; frontier cadence ≤ 1 s [834 ms]; daemon
  RSS ≤ 256 MiB [runner 18.76 MB]; `goalcup start` RSS is not sampled; chain bytes within the reservation [188 423 843 B, 35.1 %]; load
  tick lateness < 1 s [95 ms]; loss in the stall [12 288 B; up to about 2 MiB of payload per
  discarded epoch here]; the matcher's `lead`/`trail` offsets.
- **No service:** no devserver or `--dev-key`; `anchor: local_only` and no receipt asserted.
- **Privacy:** the dead daemon's control token is removed before retaining its config; scans cover
  the entire run root, including config, home and workspaces, and the evidence directory.
- **Resolution:** native ages use whole seconds; the 1 s status poll cannot resolve E02's 834 ms sync cadence.
- **Pre-fix smoke history:** two initial 25 s runs; one filled the queue 18.6 s into the stall,
  after the sparse phase began, and the identical-body matcher cascade (E02 §8.1) failed.
  Smoke 2's dense traffic stopped 18 s into the stall and had no Recovery; both remained failed.
- **Evidence:** `spikes/m2-first-release/out/t23/<run>/` (git-ignored): `identity.txt`, `verdict.json`,
  `crosscheck.json`, `v1/`, `verify.json`, `figures.json`, `samples-summary.json`, the run root.
  Epoch discard can leave `reobserved_ids_opened_under_lost` at 0 despite fresh re-observations.
## 4. Historical D3 checkpoint and current measured Part B
- D3 candidate `b2d1991aec646d591b8c05b906aeba82d10e7fb5`: four-command gate exits 0/0/0/0; workspace 2,395/0/21 ignored (134 groups); feature-aware vertical 8/8, 183.72 s, no reruns.
- Reservation oracle passed: at first workload act `st_blocks × 512 ≥ 67,108,864`, EOF < 1 MiB; status quota/reserved = requested; 2^60 refused before launch. This is the asserted bound, not an exact allocation count.
- Original sparse measurement (`e9a7b64b`, `target/t23-evidence/vertical-all-2.log`): capture→append 187.6 ms; append→durable seen 1,148 ms by 0.5 s polling. Guards < 2 s / < 5 s passed; not a new D3 latency measurement.
- Fix smoke `load-smoke-20260930T050849Z`: **FAILED**, 40.058 s stall, one QueueFull Recovery,
  2,228,224 B loss, 15,603,831 / 536,870,912 chain B. Six checks/CP-A4/no-service/verifier passed; privacy failed on copied helper-source regexes. Failed evidence unchanged; narrow cleanup diagnostic passed.
- Final driver deletes only its two generated v1 helper copies after both cross-checks, then scans all runtime evidence; frozen commit/hashes retained. No second timed smoke. Inherited future-Rust warning disclosed; no dependency change.
- Release readiness at build SHA `b2d1991`: both builds exit 0, arm64/version 0.1.0, test-support upstream-root marker present. Full Part B was unrun at this historical b2 readiness checkpoint; actual source3c results follow.
## Full Part B result — one synthetic run on frozen 3c

`bash spikes/m2-first-release/t23/run.sh load` once: shell 0/driver PASS/all 18 assertions true. Run `load-20261001T071828Z`, clean source `3c542aa57c765af6869094ee86c0cf78467a552a`; prepared four binary hashes matched. Runtime start 2026-10-01 07:18:30 UTC, driver cleanup complete 07:48:44 UTC, outer terminal not separately timestamped. Planned workload 1,800 seconds, measured 1,809.456 seconds (30m9.456s); quiet lease 07:18:28–07:58:28, final snapshot/release 07:59:37. No workload extension to fill the quiet lease. Startup, 07:36:39 and final process snapshots found cargo/rustc/hdiutil/cc/clang/ld absent; these are snapshots, not continuous monitoring. Startup load 6.40/7.37/6.88, sampled range 4.66–24.33, final 7.50/7.02/8.00; host load rose.

All six corrected checks PASS: `sent_equals_forwarded`, `sent_equals_retained_outside_gap`, `gap_is_visible_and_recovered`, `native_matches_file`, `no_leak`, `load_self_check`. CP-A4 exercised the allowed unrecorded-epoch branch: first Recovery names a lost zero-record epoch, not the first retained wire producer; two connections reopen under fresh producer; no clean lost-producer close. Timer resumed daemon after 40,070 ms, with one queue_full Recovery stamped inside stall; four producer_lost gaps (three stream, one streamless). Corrected matcher 0.05s/no clean unmatched or missing outside window, Open-start offsets −31.6 to +61.9ms. Frozen v1 (7689ea5, 1s) remains FAILED on exactly gap/recovery and retained-outside-gap checks, with two misses (32,768-byte SSE response and zero-byte SSE request).

Verifier exit 1 is expected: incomplete, sealed, no torn tail, producer_recovered, LocalOnly; LocalOnly/no ACK/receipt/receipt clock, no configured remote service/devserver/devkey, required loopback upload port closed. Absence of attempted upload dials was not independently measured in this full run; earlier lifecycle dial-trap test is separate evidence. Corrected privacy scan 73 files/zero hits; final retained-evidence scan 85 files including config/home/workspaces, zero file/credential/sentinel hits under the prescribed native-source fixture exception: synthetic native source holds the sentinel by design; retained chain/logs do not. This is not a claim that every raw fixture file lacks the sentinel. Driver only removed documented two copied helper sources/transient control token and relocated run root into ignored evidence; no manual cleanup. Original edited smoke load-smoke-20260930T050849Z remains FAILED, with its privacy-source-copy false positive; two earlier 25s smoke failures also preserved.

Logical input 125,706,240 bytes, retained 120,893,399; **4,812,841-byte measured difference/loss**, no loss bound. Endpoint bad requests/TLS failures/interruptions/unknown requests or bytes/aborted connections all 0; shutdown in deadline. Chain 180,634,718 bytes / 536,870,912-byte reservation (33.65%), 27,474 records. Physical reservation release still T25 deferred; no reclamation/cleanup.

Wire outside stall: n=10,649, p50 11.4ms/p99 201.2ms/p999 218.8ms/max 349.7 ms, 3 over 250 ms; settled n=10,535/p50 11.6 ms/same p99/p999/max. Native outside stall: n=1,762/p50 638.0 ms/p99 777.9 ms/p999 1,085.9 ms/max 40,577.9 ms, all over 250 ms; 30 held-secret lines max 1,097.4 ms. Whole-second native timestamps preclude blanket 250 ms claim. Durable-frontier max change interval 2.008 s across 1,773 status samples; 1s resource polling 1,806 samples/max gap 1.432 s cannot resolve E02 834ms cadence. Start RSS NOT SAMPLED. Daemon time-l max RSS 13,565,952 bytes; runner sampler 13,484,032 / workload-tree 0. No general latency/cadence/RSS guarantee.

Echo/WS/SSE 900 each max lateness 0.061 s; native 1,800 max 0.062 s, none over 1 s. Six 300 s phases DSDSDS, in-process proxy one worker vs E02 four. E02 comparison limited to six checks/verifier, fixed-timer stall/loss reported separately. Synthetic own-CA/test-support target evidence is separate from real-harness/shipping-prefix/production delivery or long-unattended qualification and proves no network-cause remediation.

Evidence: integration `target/t23-PartB-evidence/T23-PartB-run-report.md`, final reviewed report SHA `1360f6a46fc84a5def66ba262b49feb5225518f36bb379c8efb60d260334c2e4`, raw ignored `spikes/m2-first-release/out/t23/load-20261001T071828Z/` including identity/verdict/crosschecks/v1/verify/figures/samples/run root. `06-report-corrections.txt` preserves linkage from original completion-marker report SHA `0c5d525cff605a2880420c3a29d7b88264fb4ef3324522a9efc105401aec3de4` and interim `d591d4cf7828334550ffe90d5606caf00cb29c1721db85ee6e5fbb388f995265` to final report. Wording-only corrections, raw verdict/data and original marker unchanged; no runtime/test/build repeat. Both scoped correctness/quality reviews approved; G-2 COMPLETE with this final docs commit. Future tests remain ledger backlog, no added campaigns.

| Quantity | Measured source3c | Target [E02], limits retained |
|---|---|---|
| Run / tree / quiet window | load-20261001T071828Z /3c542aa5 /07:18:28–07:58:28UTC | actual workload1,809.456s |
| Corrected six checks | all PASS | all ok |
| Frozen v1 | 2 failed predicates/exit1 | reported unchanged |
| Capture-integrity | exit1/incomplete/sealed/LocalOnly/4gaps/no torn tail | expected incomplete |
| Wire outside-stall max |349.7ms;3/10,649over 250 ms|250ms [237.2ms]|
| Native outside-stall max |40,577.9ms; whole-second timestamps|no blanket 250 ms proof|
| Durable-frontier max interval |2.008s /1,773polls|1s [834ms], poll limits|
| Daemon maxRSS |13,565,952B; startRSSunmeasured|256MiB [18.76MB]|
| Chain/reservation |180,634,718 /536,870,912B (33.65%)|[188,423,843B/35.1%]|
| Schedule maxlateness |wire0.061s/native0.062s|1s [95ms]|
| Input-retained difference/loss |4,812,841B|reported [12,288B], no bound|
