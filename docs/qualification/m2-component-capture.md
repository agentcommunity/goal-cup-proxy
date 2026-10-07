# E00: experimental real-component capture checkpoint

> **Historical (2026-10-02).** E00 ran a non-shipping example harness, not the shipped path. Current: [m2-synthetic-vertical.md](m2-synthetic-vertical.md); index: [README](README.md).

> **What this is:** a controlled, loopback-only experiment, run 2026-09-27 on branch
> `m2/e00-capture`. A non-shipping Rust example composes the real proxy, streaming redactor,
> native adapter, v5 writer and integrity reader, and drives them with a controlled TLS
> endpoint and an external traffic generator.
>
> **What it is not:** evidence of real harness support, an installed or reserved pilot, a
> signed anchor, or delivery.
> - Storage is **experimental, unreserved quota-only accounting**: `physical_reservation=false`,
>   `reserved_bytes=0`. The quota is an admission ceiling, never a reservation.
> - The anchor is `local_only`. No receipt, ACK, devserver or upload exists.
> - Every number below was measured on this host in the runs named. None is copied from a target.

## 1. Verdict

| Item | Result | Evidence |
|---|---|---|
| Named tests (packet) | **pass**, together with 15 more tests of this file; 20/20 in debug on `849a393` (18/18 on the measured `a4c9617`) | `cargo test -p goalcup-cli --offline --test m2_component_capture`, §5 |
| Two-minute controlled sequence (release) | **ran to completion; capture matches the traffic; loss is visible and honest** | run 3, `20260927T171934Z`, §3 |
| Library integrity exit | **1** (`incomplete`, sealed, one deliberate gap and recovery) | `summary.json` `integrity.exit_code` |
| Literal `goalcup-verify --capture-integrity` exit | **1** | `verify.stderr`, `identity.txt` `verify_exit=1` |
| Physical connection identity across the epoch change | **kept**: 3 connections were re-observed under the fresh producer, 2 of them with IDs first opened under the lost producer | `crosscheck.json` |
| Fabricated assistance | **none**: 0 `Assistance` records | `facts.json` |
| Prospective targets | flush interval, sync interval, queue, recorder RSS and analysis RSS are all within target in runs 2 and 3 (§3.4). Run 1 overshot flush and sync by 10 ms and 32 ms, and the schedule was fixed | §3.4 |

The expected integrity exit is 1 because the stall is a deliberate, known loss. Per the packet,
"a deliberate gap case expects 1". A clean run of the same components without the stall exits
0: see `local_finish_without_service_is_complete` (library exit 0) and the release CLI smoke in §5
(runner exit 0, literal `goalcup-verify` exit 0).

## 2. Candidate, host, commands

- **Candidate (code):** `a4c9617ed715fde92c28d12b51f6a5894ecf9674`. It contains m2/wire `87d7f96` (T00, T01, T02, T03, T04, T05, T09, T10, T11, T13, T18, T00-F2) and the E00 example.
  - At run time the only uncommitted file was this document (`status_clean=no`).
  - This document is committed after the run. The code is unchanged.
  - Earlier runs are kept as evidence:
    - Run 1 (`99a631d`) had the old flush and sync schedule (§3.4).
    - Run 2 (`dce0fe7`) had the current schedule. Its code differs from run 3 only in how a relative `--out` path is handled and in one summary field.
- **Host:**
  - macOS 26.6.2 (25G83), Darwin 25.6.0, arm64 (Apple M4 Pro, 12 cores, 24 GiB).
  - Filesystem: APFS on `/dev/disk3s1` (`/System/Volumes/Data`: apfs, local, journaled).
  - Toolchain: rustc 1.98.0 and cargo 1.98.0 (Homebrew), **release** profile.
- **Binaries (sha256):**
  - `target/release/examples/m2_capture_experiment`: `c2a52b149fdb0af44ef6f19338447cf00007ce9321a8c031af8af82b6ec70207`
  - `target/release/goalcup-verify`: `6df96af721edc85b18dd9e7698bb0a72d701ca5359be87d069a736267f31bd89`
- **Network:** loopback only. `api.e00.localhost` and `auth.e00.localhost` resolve to 127.0.0.1 and ::1 on this host. The proxy reaches the endpoint through its ordinary CONNECT-target resolution, and no library change or resolver override is involved.

Literal commands (`$WT` = the worktree, `$EV` = the evidence directory, `$SHA` = the candidate):

```sh
export CARGO_BUILD_JOBS=3
cargo build --offline --release -p goalcup-cli --example m2_capture_experiment
cargo build --offline --release -p goalcup-verifier --bin goalcup-verify
EXE=$WT/target/release/examples/m2_capture_experiment
/usr/bin/time -l "$EXE" run \
  --out "$EV/run" --quota-bytes 1073741824 --goal-file "$EV/goal.txt" \
  --profile-id e00-controlled-generator --harness-version m2-e00-generator-1 \
  --recorder-revision "$SHA" --continuation "none: one fixed plan, no follow-up input" \
  --capture api.e00.localhost:/v1/echo:exact:e00-echo \
  --capture api.e00.localhost:/v1/sse:exact:e00-sse \
  --capture api.e00.localhost:/v1/ws:exact:e00-ws \
  --capture api.e00.localhost:/v1/unknown:exact:e00-unknown-v1 \
  --capture api.e00.localhost:/v1/bulk:exact \
  --auth-host auth.e00.localhost --upstream-roots endpoint --controlled-endpoint \
  --native-source 'name=e00,root={native_root},pattern=e00-native.jsonl,form=append' \
  --ca-env NODE_EXTRA_CA_CERTS \
  --stall-on-open-path /v1/bulk --stall-max-ms 30000 --workload-timeout-secs 300 \
  -- '{self}' generate \
    --endpoint '{endpoint}' --auth-endpoint '{auth_endpoint}' \
    --native-path '{native_root}/e00-native.jsonl' --test-id '{test_id}' \
    --plan sequence --duration-secs 120 --bulk-mib 12 \
    --report '{out}/generator-report.json' --workspace '{workspace}' \
    --upstream-ca '{upstream_ca}'
/usr/bin/time -l "$WT/target/release/goalcup-verify" --capture-integrity "$EV/run/trace"
"$EXE" facts "$EV/run/trace"
```

The goal file holds 154 bytes: `E00 controlled workload: exchange the planned traffic through the recording proxy for two minutes, then write result.txt containing exactly m2-capture-ok.`

The whole procedure is `target/e00-evidence/qualification/run.sh`. The independent check is `target/e00-evidence/qualification/crosscheck.py`. It reads `chain.jsonl` directly, decodes each segment's base64 and compares the result with the generator's report and the native source file. Evidence is not in git: `target/e00-evidence/qualification/20260927T171934Z/` (run 3), `.../20260927T171307Z/` (run 2) and `.../20260927T170859Z/` (run 1).

## 3. The two-minute controlled sequence (run 3, `a4c9617`)

### 3.1 What ran

The generator was the workload: an external process launched with only `HTTPS_PROXY`,
`https_proxy` (the proxy URL with its credential) and `NODE_EXTRA_CA_CERTS` (the per-launch
proxy CA) in its environment. Every connection went through the real proxy.
- **Per 1 s iteration**, over one keep-alive HTTP connection and one WebSocket, both long-lived and renewed every 10 and 15 iterations:
  - an 8 KiB echo;
  - a paced 5-event SSE response;
  - an unknown-protocol binary exchange every 20 iterations;
  - a fragmented text echo (700-byte fragments with an interleaved ping) and a binary echo;
  - a 3-message server stream every 5 iterations;
  - two concurrent extra connections (SSE, then a WebSocket echo).
- **Native transcript:** one JSONL line per iteration, ending in a partial line.
- **Mid-run:** one 12 MiB upload plus a 12 MiB download on its own connection. This is the stall trigger.
- **Auth host:** one request at the end.

| Measured | Value | Source |
|---|---|---|
| Workload duration / exit | 120 761 ms / exit 0, `AlreadyStopped` (reaped, group empty, no signal) | `summary.json` `workload` |
| Runner duration | 121 091 ms (121.31 s real) | `summary.json`, `runner.stderr` |
| Connections accepted / intercepted / auth tunnel | 498 / 497 / 1 (all 497 `closed_complete`) | proxy `health` |
| Requests at the endpoint | 730: 119 echo, 357 SSE (1 547 events), 6 unknown, 1 bulk, 1 auth, 246 WS upgrades | endpoint counters |
| WebSocket | 246 sessions, all clean closes; 1 048 messages (500 client, 548 server), 905 fragmented, up to 5 frames | generator report |
| Generator self-check | `ok: true`; every echo, SSE stream, unknown body, WS message and the bulk transfer was byte-checked on the client; `result.txt` = `m2-capture-ok` | `generator-report.json`, `workspace/result.txt` |

### 3.2 Capture against traffic (independent cross-check)

| Check | Result |
|---|---|
| Non-bulk HTTP exchanges (482), request and response bodies by path + sha256 | **equal** as multisets to the generator's own digests |
| WebSocket messages, 500 client + 548 server, by sha256 | **equal** |
| Captured bytes vs endpoint's independent counters | WS 581 865 in / 689 553 out = endpoint `ws_bytes_in/out`; responses 1 291 776 = echo 974 848 + SSE 298 928 + unknown 18 000 |
| Native stream | one stream, **equal** to the 9 948-byte source file (sha256 = generator's), which ends in a 29-byte partial line; ended `complete` at finish (finish-time length, not a semantic claim) |
| Unknown protocol | hint `e00-unknown-v1` retained on request and response; no decoder involved |
| Redaction | **0 bytes redacted**: 0 audit spans over 2 030 segments; retained = source for every source. The traffic is words by design, so the fail-closed path was **not exercised** by this run (§7) |
| Goal and launch | 1 `Launch` naming the goal producer; the goal stream is the 154 goal bytes; `profile_digest` = sanitized recipe digest (no values, no goal, no credential) |

### 3.3 Deliberate queue stall and recovery

| Measured | Value |
|---|---|
| Stall | the owner stopped draining at the bulk upload's `Open` (`/v1/bulk`), started 17:20:35.163Z |
| Released by | the sink refusing the proxy producer with `QueueFull` after **20 ms** (`refused_queue_full=1`) |
| Queue high-water | payload **8 205 195** encoded bytes (payload budget 8 257 536 = 8 MiB − 128 KiB control reserve), control 512, 134 items |
| Forwarding | unaffected: the 12 MiB each way was relayed and byte-checked by the client |
| What was retained of the bulk | a 2 179 071-byte request prefix, then `Gap{queue_full}` from the interrupted redactor, **before** the Recovery record; no End; the bulk response (sent during the failed epoch) is not in the chain |
| Recovery | 1 `Recovery{previous: lost producer, reason: queue_full}` at the next request boundary; 0 `Pending` answers; the fresh producer re-observed 3 connections (`already_open: true`), 2 of them with IDs opened under the lost producer, 1 first opened during the failed epoch. The lost producer had 3 connections open; the third was the bulk connection, which closed during the failed epoch, so it is the only intercepted connection with no `Close` in the chain |
| Producer closes | goal (final_seq 2), native (final_seq 119, = the adapter's own report), fresh proxy producer (final_seq 4 664); the lost producer is never called closed |
| Proxy stop | clean drain (`drained`, 0 unterminated, close `queued`), `capture_loss=true` (the recovered loss: 1 sink refusal, 2 control refusals), 0 transport failures; 0 stream-less gaps (no fail-open tunnel, no shutdown cut) |
| Integrity | library and binary exit **1**: `integrity=incomplete`, `sealed=true`, `coverage_incomplete=true`, `torn_tail=false`, `gap_count=1`, findings `gap_recorded`, `producer_recovered`, `anchor=local_only`, `key_source=none`, 8 293 validated records (13 516 875 bytes); terminal `local_finish` with the reaped `AlreadyStopped` |

### 3.4 Resources and cadence (measured; prospective targets from the spec)

| Quantity | Run 3 (`a4c9617`) | Run 2 (`dce0fe7`) | Run 1 (`99a631d`) | Prospective target |
|---|---|---|---|---|
| Owner safe-byte flush, worst interval (count) | **210 ms** (595) | 210 ms (595) | 260 ms (476) | 250 ms |
| Durable frontier sync, worst interval (count) | **831 ms** (150) | 831 ms (150) | 1 032 ms (122) | 1 s |
| Capture queue high-water in the sink (encoded payload max + control max; see note) | **8 205 707 B** | 8 205 524 B | 8 205 579 B | 8 MiB (8 388 608 B) |
| Recorder (runner) peak RSS, `getrusage(RUSAGE_SELF)` | **22 249 472 B** (21.2 MiB) | 22 986 752 B | 22 675 456 B | 256 MiB |
| Runner peak memory footprint (`/usr/bin/time -l`) | 9 044 520 B | 9 863 744 B | 9 912 896 B | — |
| Workload (generator) peak RSS, reaped child | 34 881 536 B | 34 242 560 B | 34 865 152 B | — |
| Analysis: `goalcup-verify --capture-integrity` peak RSS / time / input | **4 980 736 B** / 0.66 s / 8 293 records | 4 964 352 B / 0.48 s / 8 303 | 4 964 352 B / 0.61 s / 8 258 | 512 MiB |
| Chain size / quota / reserved | 13 516 875 B / 1 073 741 824 B / **0** | 13 524 432 B / same / 0 | 13 621 580 B / same / 0 | — |

In run 1 the owner scheduled its flush and sync at exactly 250 ms and 1 s, and the
measured worst intervals overshot both by the loop's granularity. The schedule is now 200 ms and 800 ms
(`dce0fe7`). Runs 2 and 3 each measured 210 ms and 831 ms. All three runs are otherwise
equivalent: same exits, same cross-check results. Run 1's bulk prefix was 2 277 373 bytes, and
runs 2 and 3 each captured 2 179 071.

**About the queue:** the C2 bound is on *encoded* bytes, and a `Bytes` frame's data is a JSON
number array (about 4× its size). The 8 MiB queue therefore holds about 2 MiB of payload data:
the sink admitted 5 739 216 bytes of data as 24 909 122 encoded bytes over run 3. This is faithful to the
contract, and the CP1 reviews record it (M4). For the same reason the native adapter polls at
most 4 × 64 KiB raw per poll (about 1 MiB encoded), not T05's 1 MiB default (CP1 review 3 M1).

### 3.5 Artifact identities (run 3)

```
d7e9581c8c6f117095b039646eb1832f2d33e23676304abc61484d42c012a4cc  trace/chain.jsonl
cc6db9c12e296859b567d6951133802843702d9f9b8b5327693766833b13eeee  trace/seal.json
50ddda539f19e0cae40f7a8fa205678e076a7b97e328a85bfca29912cb9f47c0  trace/baseline.json
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  trace/acks.jsonl (empty)
e13befa2b430073fedba555b7b99774b886103c59d3b00d033264b9122192b58  summary.json
5d84cdab3e81603bdb433d08694562d58c4448da98f757e4a88b74829fb5dc7e  startup.json
be17bebcafb450607e34c62b02055e9612c74b6330718a7dbe264303cebaecf6  integrity.json
82dadf26a6d4b6c5dff2c4d7eef2a2cb4acdf540398dede69daf94095d896ea6  generator-report.json
```

Durable frontier at finish: seq 8292, head `8f24c9dc41e264a51e2c175087a80a703cd3ba75d25ef300f87e87db114019b4`,
13 516 875 chain bytes. The local finish is the terminal. The startup and final summaries both state
`experimental=true, physical_reservation=false, reserved_bytes=0, quota_bytes=1073741824,
signed_anchor=false, anchor=local_only`.

## 4. What the components are, and what E00 owns

- **Real, unmodified components:**
  - `goalcup_cli::proxy::{start, ProxyHandle::stop}` (T13, composing T09 admission and CA, the T10 HTTP relay and the T11 WebSocket relay);
  - `goalcup_daemon::redact::stream::StreamRedactor` (T04, fail-closed past the 16 KiB hold);
  - `goalcup_daemon::native_capture::NativeCapture` (T05);
  - `goalcup_protocol::export::TraceExport` with `configure_capture_quota`, `append_chain`, `sync_capture` and `write_local_finish` (T03);
  - `goalcup_verifier::capture_analysis::reader` (T18).
- **E00-owned, non-shipping (`crates/goalcup-cli/examples/`):**
  - the bounded direct `CaptureSink` (`sink.rs`);
  - the single entry owner (`owner.rs`);
  - the chain writer (`chain.rs`);
  - the controlled TLS endpoint (`endpoint.rs`);
  - the external traffic generator (`generator.rs`);
  - workload spawning and reaping (`workload.rs`);
  - the runner lifetime (`runner.rs`);
  - chain facts (`facts.rs`).

Contract conformance of the E00-owned parts:

- **Sink.** It is a C2 receiver. It checks the following before admitting anything:
  - Ready, Pending, RetryFresh and Disabled admission. Every proposed ID, Pending ones included, counts toward `MAX_PRODUCERS`.
  - Sequence continuity and stream transitions.
  - Contiguous offsets: the first Bytes at 0, and a forward jump only right after a Gap (CP-A3).
  - The fresh-epoch Recovery rule.
  - The aggregate `MAX_OPEN_STREAMS`. A lost producer's streams count until a Recovery supersedes it.
  - An exact `final_seq` close.

  Payload is bounded in encoded bytes, with a separate control reserve. A refusal latches the producer lost and queues one loss marker behind everything that producer had admitted. Frames that arrive after `close_intake` are refused without growing the queue. No redaction or I/O happens under the sink lock.
- **Owner.**
  - One redactor per producer.
  - A lost producer's redactor is interrupted, and its output appended, before any Recovery naming it (F6).
  - The 250 ms flush and 1 s sync are owner-driven.
  - `ProducerClosed` carries the owner's clock and the validated `final_seq`, and is never written for a lost producer.
  - Stream-less gaps are coalesced to one per reason:
    - `tunneled_fail_open` → DecodeFailed;
    - `cut_unsettled`, `unterminated` and `accept_loop_lost` → ShutdownDeadline;
    - T05 discovery facts → LimitExceeded, SourceChanged or ShutdownDeadline;
    - a required native source with no `Open` → SourceChanged.
- **Chain writer (CP-A1).**
  - The first payload `CaptureStorageFull` makes payload discard sticky.
  - Once any segment of a stream has been discarded, every later segment of that stream is discarded too.
  - End and Gap are written only for streams the chain opened. UpgradedWebSocket and Close are written only for connections the same producer opened or re-observed.
  - After any terminal-allowance refusal, nothing more is appended, and the local finish may still be attempted.
- **CP-A2 (T00-F2).** It is in the candidate.

## 5. Tests

`cargo test -p goalcup-cli --offline --test m2_component_capture` (debug profile) gives **20 passed**
on `849a393`, and 18 on the measured `a4c9617`. The fix round added the two exit-code tests. The end-to-end tests run the real runner with the generator as the workload. The
test binary re-executes itself as `generator_entry`, with the generator's argv in
`M2_E00_GENERATOR_ARGV`.

| Test | Oracle |
|---|---|
| `real_components_capture_http_ws_and_native` | Every HTTP exchange and WS message, by digest, equals the generator's report; totals equal the endpoint's independent counters; the native stream equals the source file; exit 0; `result.txt` is checked outside the recorder; the auth host is never captured; Launch and goal are present; all three producers are closed |
| `boundary_secret_absent_from_experimental_chain` | Synthetic secrets are split across request chunks, SSE chunks, WS fragments and native appends; two **interrupted prefixes** (an HTTP body and a WS message cut mid-secret); a **24 KiB overlong value past the 16 KiB hold** (fail-closed); a secret in an auth-tunnel body; and the proxy credential. None appears in any retained or diagnostic file (raw), in any decoded segment, or in the runner's output. Each carrier shows an `anthropic_key` audit span, and the 24 KiB value was replaced. Exit 0 (peer interruptions are Interrupted ends, not capture loss) |
| `unknown_protocol_and_partial_native_line_are_retained` | Binary non-UTF-8 bodies are byte-exact under hint `e00-unknown-v1` (recomputed from the test ID, not read back). The unterminated last native line is retained at contiguous offsets; `End{complete}` means finish-time length only |
| `experimental_quota_never_claims_reservation` | Quota = 1 MiB terminal allowance + 256 KiB. The startup and final summaries say `reserved_bytes=0`, `physical_reservation=false`; chain blocks > 0 and length ≤ quota; forwarding survived (the workload checked every echo); loss is coalesced `StorageFull` gaps; sealed; exit 1 |
| `local_finish_without_service_is_complete` | Library exit 0: `valid`, `local_only`, `key_source=none`, no receipt, empty ACKs. The terminal is the T03 LocalFinish bound to `seal.json` by sha256, with the reaped `AlreadyStopped` termination. `final_seq` equals 2 for the goal and the adapter's own value for native |
| `deliberate_queue_stall_recovers_under_a_fresh_epoch` | Debug-sized version of §3.3 (6 s, 9 MiB bulk): QueueFull releases the stall; the lost epoch's gap precedes Recovery; IDs are re-observed under the fresh producer; exit 1; flush and sync kept running |
| `workload_not_launched_has_its_own_exit` (exit 5, unsealed, `setup_error`), `usage_errors_do_not_collide_with_integrity_exits` (64; help 0), `required_native_source_that_never_appears_is_a_visible_loss`, `layout_paths_are_absolute_even_for_a_relative_out`, `chain_writer_*` (2), `direct_sink_*` (2), `emitter_over_direct_sink_recovers_after_queue_full`, `native_adapter_over_direct_sink_closes_with_a_validated_final_seq`, `proxy_stop_facts_map_to_streamless_gap_reasons`, `workload_termination_comes_from_real_reaping`, `direct::controlled_endpoint_serves_every_route_and_counts_independently`, `generator_entry` | Component and seam oracles |

Mutation checks (evidence in `target/e00-evidence/phase{1,2}-mutations/`):
- 9 sink and chain mutants, on `c640e3b`;
- 7 owner and runner mutants, on the phase-2 code, rerun after two oracles were strengthened;
- the CP-A1 sticky-terminal mutant.

Two mutants survive, and neither is counted as killed:
- **M4**, dropping the `!discarding` guard in the chain writer, is **equivalent**: `admissible()` already discards those segments.
- **O8**, the original native finish with a zero budget (`NativeFinishBudget { max_bytes: 0, max_duration_ns: 0 }`), **survived**. Rerun on the fix-round code it still passes 18/18 (`phase2-mutations/summary3.txt`).
  - It survives because T05 runs at least one bounded drain pass whatever the budget, and E00's native transcripts (≤ 10 KB) fit in one pass. The budget value cannot be observed at E00's size.
  - The property the mutant was meant to cover, that native finish happens only after the workload is reaped, is covered by a different two-edit mutant (finish before the workload spawns). That mutant is killed by 5 tests (`summary2.txt`).
  - No E00 test pins the finish budget itself.

A release CLI smoke of the external `run` mode used the basic plan with a *relative* `--out`
(`target/e00-evidence/qualification/smoke-basic-a4c9617/`). Runner exit 0; literal
`goalcup-verify --capture-integrity` exit 0; the cross-check matched every exchange and message.
An earlier smoke on `dce0fe7` with the same relative path exited 1: T05 refused the relative
native root. That run is kept as `smoke-basic-dce0fe7/`. The trace was honestly unsealed, with a
`SourceChanged` gap for the required native source. `a4c9617` makes run paths absolute and
reports the setup error.

Fix round 1 (`849a393`) changed only exit codes and helper deduplication; capture behaviour is
unchanged. It was re-smoked in release (`smoke-basic-849a393/`): runner exit 0; literal verify
exit 0; the cross-check matched; `run --bogus` exited 64 and a missing mode exited 64. The
two-minute run in §3 stays the measurement on `a4c9617`.

## 6. Runner interface for E01 and E02

```text
m2_capture_experiment run [ARGS] -- COMMAND [ARGS...]
m2_capture_experiment generate [GENERATOR ARGS]
m2_capture_experiment facts TRACE_DIR
```

`run` arguments:

| Argument | Meaning |
|---|---|
| `--out DIR` | private run directory; must not exist; created 0700 with `trace/ workspace/ native/ ca/`; made absolute before any placeholder is filled |
| `--quota-bytes N` | unreserved experimental quota, including the 1 MiB terminal allowance |
| `--goal-file PATH` | read once (UTF-8, ≤ 64 KiB); retained only as the redacted Goal stream; available to the command as `{goal}` |
| `--profile-id`, `--harness-version`, `--recorder-revision`, `[--continuation]` | Launch record strings (≤ 1 KiB, no control characters) |
| `--capture HOST:PATH:exact\|prefix[:HINT]` | repeatable; HOST becomes an approved (intercepted) host; first match wins |
| `--auth-host HOST` | repeatable; always an opaque tunnel |
| `--upstream-roots webpki\|endpoint` | **E01: `webpki` (real hosts)**; E00: `endpoint` (the controlled CA) |
| `--controlled-endpoint` | E00 only |
| `--native-source name=N,root=DIR,pattern=REL,form=append\|snapshot[,hint=codex-jsonl\|claude-code-jsonl][,scope=ID][,required=true\|false]` | T05 source; `{id}` in the pattern is filled by `scope` (validated by T05); `required` (default true) makes "no file ever opened" a visible loss |
| `--ca-env NAME` | repeatable; `NAME=<proxy CA certificate path>` in the workload env. Codex: `CODEX_CA_CERTIFICATE`; Claude: `NODE_EXTRA_CA_CERTS` |
| `--workload-env KEY=VALUE`, `--workload-env-inherit KEY` | the rest of the workload env (placeholders substituted), e.g. `NO_PROXY`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR`, `HOME` per the T02 recipes |
| `--workload-stdio files\|inherit` | `files`: `{out}/workload.stdout` and `.stderr`; `inherit`: direct harness output |
| `--workload-timeout-secs` (600) | after this, SIGTERM the group, wait 10 s, SIGKILL, wait 5 s |
| `--startup-deadline-ms` (10000), `--shutdown-deadline-ms` (10000) | proxy and endpoint bounds |
| `--stall-on-open-path PATH`, `--stall-max-ms` (30000) | E00's deliberate stall only |
| `--test-id ID` | shared endpoint/generator ID (default: the run UUID) |

Placeholders:
- in `--native-source` roots and scopes, `--workload-env` values and the command: `{self}`, `{out}`, `{workspace}`, `{native_root}`, `{endpoint}`, `{auth_endpoint}`, `{upstream_ca}`, `{ca_path}`, `{test_id}`, `{run_uuid}`;
- in command arguments only: `{goal}`.

**Workload environment:** `env_clear`, then exactly:
- `HTTPS_PROXY` and `https_proxy` = `http://goalcup:<secret>@127.0.0.1:<port>`. The secret is in memory only and never appears in argv, a file or a log;
- each `--ca-env` name set to the CA certificate path;
- `--workload-env`;
- `--workload-env-inherit`.

Stdin is `/dev/null`, the working directory is `{workspace}`, and the workload runs in its own process group.

**Lifetime:**
1. Export: quota, then the v5 baseline.
2. Owner, which appends the Launch record first.
3. Goal stream, appended and synced before launch.
4. Controlled endpoint (E00 only).
5. Proxy.
6. Native adapter.
7. `startup.json` (the disclosure).
8. Workload spawn. While it runs, `health.json` is published read-only every 1 s, and fail-open tunnels become gaps.
9. The workload and its group are reaped.
10. Native finish.
11. Proxy stop (one deadline).
12. Endpoint shutdown.
13. Sink intake closed.
14. Owner drain: stream-less gaps, then interrupts, then sync.
15. Seal manifest.
16. `write_local_finish` with the reaped termination.
17. T18 check.
18. `summary.json` and `integrity.json`.

SIGINT or SIGTERM to the runner is the bounded stop: the workload is stopped, and the run then finishes.

**Exit table (`run`).** The runner's own codes never collide with integrity exits.

| Exit | Meaning |
|---|---|
| 0 / 1 / 2 / 3 | The workload ran. This is the T18 integrity exit of the finished trace: complete / incomplete or unsealed / invalid (torn tail included) / no usable report |
| 4 | Capture never started. An argument value, the output directory, the runtime, the trace, its quota or baseline, or the owner thread was refused. Nothing was launched; a trace directory, if any, holds at most its baseline |
| 5 | Capture started (Launch and goal are chained) but the endpoint, proxy, native adapter or workload could not be started. The trace is left **unsealed** and still checked. `summary.json` holds `setup_error` and the integrity result (typically 1); stderr names the reason |
| 64 | The command line did not parse (`--help` exits 0) |

`generate` exits 0 when every check passed, 1 when one failed, 3 on an I/O failure and 64 on its own usage error. `facts` exits 0, 3 when the trace cannot be read, and 64 without an argument.

Always record `goalcup-verify --capture-integrity {out}/trace` separately; its exit is the canonical integrity exit.

**E02** consumes the merged runner read-only.

## 7. Limitations and open items

- **Synthetic scope.** Controlled endpoint and synthetic generator only. The auth tunnel, NameConstraints acceptance and proxy-auth-from-URL are proven only for this client; the harness rows stay E01's.
- **Launch/goal linkage.** T18 does not link `Launch.goal_stream` to the goal stream (CP1 M3). E00 asserts it in its own tests.
- **Encoded-queue data window** (§3.4). The queue holds about 2 MiB of payload data. A large burst recovers rather than queues.
- **Resolver dependency.** The controlled endpoint relies on `*.e00.localhost` resolving to loopback.
- **Fail-closed redaction was not exercised by E00's runs.** The two-minute runs and the release smoke redacted **0 bytes**: 0 audit spans, and retained bytes equal source bytes for every source. The reason is that in `bec5162`, after the fail-closed T04 merged and before any qualification run, the generator's payloads were switched from long single-token runs to space-separated words. Those long runs had been replaced wholesale, which broke E00's byte-exact capture oracles, and no chain from before the switch was kept.

  So E00 shows nothing about how much realistic traffic fail-closed redaction removes. The only E00 exercise of that path is the boundary-secret test, where a keyword plus a 24 KiB run was replaced (153 of 24 759 B retained, per the reviewer's check).

  **T04's fail-closed behaviour on realistic traffic** (`goalcup_daemon::redact::stream`; ruling T04-C1):
  - A `"` followed by a run of `[A-Za-z0-9_-]` longer than 256 bytes is a candidate secret JSON key, and stays undecided until the closing quote.
  - Long hex runs (as e-mail-local-part candidates) and a PEM `BEGIN` without its `END` are also held.
  - If a held candidate reaches the 16 KiB hold, the replacement runs from the earliest undecided byte to the next newline. For single-line JSON that means **the rest of the body or WebSocket message**, later plain-text content included.
  - Standard base64 (with `+` and `/`) and prose do not trigger it.
  - The trace stays honest: every replacement is audited and nothing leaks. But what can be inspected is less than the traffic.

  **Reviewer probe, not an E00 measurement.** Numbers are from `e00-review-1-correctness.md`: real runner, proxy and T04, curl echoing single-line bodies.

  | Payload | Source B | Retained B | Replaced | Trigger |
  |---|---|---|---|---|
  | 8 KiB lowercase run | 8 192 | 8 192 | 0 | none (below hold) |
  | 32 KiB lowercase run | 32 768 | 16 | 100% | `email` |
  | Responses-shaped JSON: 50 KB text, 150 KiB standard base64, 40 KiB base64url `encrypted_content`, then a 20 KB message | 316 054 | 255 049 | 19%: everything from `encrypted_content` to the end, the later message included | `generic_secret_assignment` |
  | JSON with a 20 KiB hex string, then a 30 KB message | 70 636 | 20 124 | 72% | `email` |
  | JSON tool output with a PEM `BEGIN` and no `END`, then a 40 KB message | 47 851 | 5 130 | 89% | `private_key` |

  A Codex request that replays a reasoning item's `encrypted_content` (a base64url token) of 16 KiB or more would lose the rest of that request body from inspection.
- **E01 pre-registration.** E01 records, for every row and per stream, both:
  - **Retained vs source bytes, and audit kinds.** The `facts` mode and `summary.json` `chain_facts` already report these per source. The raw per-stream numbers come from the chain.
  - **Tool-call and tool-output inspectability.** Whether the harness's tool-call fields and tool-output fields are inspectable, and in which evidence: wire, native, both or neither.

  A request body or message that was replaced starting at a non-secret base64url or hex value is recorded as a **known limitation, not a pass**. That result feeds the open T04 ruling: whether to narrow the quoted-run key candidate, or accept the loss.
- **Queue bound vs owner backlog** (reviewer M2). The owner releases the sink's accounting when it drains a batch (up to 256 items), before redacting it. During the deliberate stall, the drained-but-unhandled rest of the stall batch is held in the owner's backlog while the sink refills.
  - The aggregate in memory can therefore exceed `MAX_CAPTURE_QUEUE_BYTES`, up to about one more queue's worth.
  - The high-water reported in §3.4 covers the sink only, and it adds the payload and control maxima, which occurred at different times.
  - Not changed in E00. T06/T15 own the production bound.
- **Secret scan of process output.** The boundary-secret test scans the runner's in-process outcome, not the example binary's real stdout and stderr. The reviewer's subprocess run scanned both and found 0 hits.
