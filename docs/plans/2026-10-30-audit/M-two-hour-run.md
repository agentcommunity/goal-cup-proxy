# Lane M: the 2-hour Codex run, root cause and fixes

Appendix to the 2026-10-30 audit, round 2. Review only: no product code was changed, nothing was built and no test
suite was run. The run report is `docs/research/results/2026-10-30-two-hour-run.md` with its evidence folder
`2026-10-30-two-hour-run/` beside it, on branch `origin/smart/tender-clarke-qupne3` (read with `git show`). It is not
on main. The run used build `a31094c` (goalcup 0.7.0). `git diff a31094c HEAD -- crates` is empty on
`smart/intelligent-newton-0avrpi`, so every `path:line` below is the code the run used. "E/" means the run's evidence
folder. Code paths are under `crates/` unless they start with `server/`, `scripts/`, `connectors/` or `docs/`. Short
forms: `sink.rs`, `owner.rs` and `writer.rs` are in `goalcup-daemon/src/runtime/capture/`; `proxy/*.rs` is
`goalcup-cli/src/proxy/`; `capture.rs` is `goalcup-protocol/src/capture.rs`; `native_capture.rs` and the daemon's
`ingest.rs` are in `goalcup-daemon/src/`. This file merges four working parts: silent loss (M1), size and upload (M2),
native capture and local logs (M3), and the plan gap check (N3). Estimates say "est." or "estimate". Anything not
checked says "unverified".

Files read for this lane (`wc -l`, inline tests included where the file has them): `sink.rs` (556), `owner.rs` (388),
`writer.rs` (391), daemon `ingest.rs` (1,356), `native_capture.rs` (1,632), `goalcup-daemon/src/redact/stream.rs`
(4,008, about half inline tests), `proxy/ingest.rs` (1,177), `proxy/api.rs` (824), `proxy/http.rs` (2,257),
`proxy/websocket.rs` (1,080), `proxy/mod.rs` (1,532), `capture.rs` (847). Tests: `goalcup-cli/tests/it/m2_capture_sender.rs`
(1,991), `goalcup-daemon/tests/it/capture_ingest.rs` (3,095), `goalcup-daemon/src/runtime/capture/tests.rs` (2,340).

## Summary

The run lost 473 of Codex's 1,261 model calls (37.5%, measured on the server copy, section 9) and every wire record
of 15 of 42 subagents, because one
piece of plumbing treated load as death. The proxy sends its frames to the daemon through two 8 MiB queues. The bytes
travel as JSON arrays of numbers, so 8 MiB holds about 2.3 MB of payload, about one and a half of Codex's 1.5 MB
full-context HTTP bodies, not five. A rate-limit retry burst from about 20 subagents overflowed the daemon queue 22
times. Each overflow dropped the whole proxy producer; the proxy read the close as a failure, and its failure budget
(10 in a row, epochs under 60 s do not reset it) switched wire capture off for good at 15:43:15 UTC. The start of
that loss is chained as an ordinary `queue_full` gap, but nothing marks it final, no recovery follows, the seal skips
the proxy's refusal counter on purpose, and the finish line said "Recording works: 1736 model calls" because it counts
at selection. Separately, a 32-slot native cap left 11 of 43 transcripts unread. The code has at least eleven other
paths to the same kind of silent or terminal stop (P1 to P25 below). The fix set is small and mostly deletion: no
failure budget (about 35 src lines removed), a per-stream gap instead of a producer drop (about 40 src), base64 for
ingest bytes (1 line), native reads that wait instead of latching (about 10 to 20 src), no native caps, a "recorded"
count kept by the chain writer plus an off-time display (about 45 src), a 2-line long-window zstd setting and a 3-line
upload backoff fix. The cut-first plan already deletes the failure budget (CH6) but files it on Sunday in PR7; it must
be the first code PR. A second 2-hour run on Sunday 1 November must prove the 14 checks in section 7.

## 1. The verified cause chain

### 1.1 What happened, step by step

| Step | What happened | Time (UTC) | Code |
|---|---|---|---|
| 1 | Rate limits: 95 calls failed `rate_limit_exceeded` (2M tokens a minute) from 15:15 to about 15:43 while 20+ subagents ran. Codex retried and moved to HTTP POSTs that resend the full context. | 15:15 to 15:43 | E/final.json, report measure 9 |
| 2 | HTTP bodies averaged 1.54 MB (375,447,818 bytes over 243 POSTs). WebSocket client messages averaged about 236 KB (158 MB over 671). | whole run | E/final.json `seg_bytes:wire_request` |
| 3 | The daemon's queue is 8 MiB, shared by every producer. The payload lane is 8 MiB minus a 128 KiB control reserve. One `OwnerSink` per entry serves the wire, native and goal producers. | | `capture.rs:50-51`, `sink.rs:7-10`, `sink.rs:109-127` |
| 4 | Ingest frames are JSON. `FrameEvent::Bytes { data: Vec<u8> }` has no serde attribute, so the bytes are an array of decimal numbers, about 3.6 bytes per payload byte (a JSON text sample: 7,768 bytes became 28,025). The sink charges the encoded size. So the 8,257,536-byte lane holds about 2.3 MB of payload. The proxy's own queue is the same size and charges the same encoding. | | `capture.rs:228-231`, encoded at `capture.rs:715-727`; `sink.rs:222-238`; `proxy/ingest.rs:120-123` |
| 5 | A frame that does not fit: the sink calls `lose(producer, QueueFull)` and answers `Disabled`. The receiver latches that producer lost, `admit_frame` returns false and `run_producer` returns, which drops the TCP stream. The owner writes a gap on every stream that producer had open, or one stream-less gap. One frame costs every in-flight stream of every subagent on that producer. | first at 15:15:12 | `sink.rs:273-282`; daemon `ingest.rs:1116-1119`, `787-790`; `owner.rs:326-341` |
| 6 | The proxy's receiver never writes, so any byte, EOF or error is a loss with no reason. `run_epoch` calls `lost()` on any read (`:1000-1007`) or failed write (`:1020-1023`). `lost()` latches `IngestLost` and calls `count_failure()`. The proxy cannot tell "dropped under load" from "daemon dead". | | `proxy/ingest.rs:14-17`, `1000-1007`, `1020-1023`, `1106-1134` |
| 7 | `STABLE_EPOCH = 60 s`. The count resets only if the epoch wrote an item and lived 60 s. | | `proxy/ingest.rs:108`, `1117-1124` |
| 8 | The connection that died at 15:39:17 had lived 161 s. The reset runs inside the same `lost()` call that then counts, so that loss is failure 1. The next nine lived 3 to 45 s each: failures 2 to 10. `MAX_CONNECTION_FAILURES = 10`, checked in `count_failure`. | 15:39:17 to 15:43:15 | `proxy/ingest.rs:104`, `463-472` |
| 9 | `latch_terminal(IngestLost)` sets `Phase::Disabled`. `next_attempt` returns `None` for `Disabled` and the writer task `run` returns. Every later `try_submit` answers `Disabled(IngestLost)`, `try_begin_epoch` answers `Disabled`. | 15:43:15 | `proxy/ingest.rs:455-460`, `891`, `861-878`, `487`, `529` |
| 10 | The emitter latches `Failed(IngestLost)`, its next `try_recover` gets `EpochAdmission::Disabled` and becomes `Disabled` for good. `open` then returns `Err`: no stream is opened, no frame is built. The relay goes on ("a `Disabled` result stops capture, never the relay"). | 15:43:15 to 16:54:43 | `proxy/api.rs:220-225`, `731-734`, `447-449`, `8` |
| 11 | The seal writes one stream-less gap per non-zero proxy counter (tunnels, fallbacks, table-full, cuts) but not `sink_refusals`: "Sink refusals are already chained as the sink's own gaps and add none". After the terminal latch they were not chained. | 16:54:43 | `writer.rs:182-201` (comment at `184-187`); `goalcup-protocol/src/control.rs:682-683` |
| 12 | `record_generation_request` counts an HTTP request at selection, before `try_recover` and before any frame. WebSocket counts every non-continuation client data frame, interrupts included (37 in this run). The finish prints that counter as "Recording works". The heartbeat's `model_requests` is the same "seen" count. | finish | `proxy/http.rs:306-320`, `364-376`; `proxy/websocket.rs:489-505`; `goalcup-cli/src/commands/start/finish.rs:405-407`, `439-444`, text `499-509`; `proxy/api.rs:342-357` |
| 13 | Status: the daemon's capture state is `Incomplete` as soon as one gap is written, and the CLI maps that to one fixed line, "the recording has gaps". There is no state for "wire capture is off now". | from about 15:15 | `goalcup-daemon/src/runtime/capture/mod.rs:1309-1313`; `goalcup-cli/src/run/mod.rs:118-131, 506`; `goalcup-cli/src/participant/status.rs:335-340` |
| 14 | Native: 32 slots (`MAX_OPEN_STREAMS / 4`) for 43 append-only rollouts. A file waits while 32 are open, and an append-only file never ends before the finish. At the finish a waiting file counts in `uncaptured_files` and the owner writes one stream-less `LimitExceeded` gap with no file and no count. E/final.json shows exactly 32 `open:native`. | 16:54:43.81 | `native_capture.rs:279`, `1555-1558`, `869`; `owner.rs:139-146` |

The frames after the latch became nothing: relayed, never encoded, never queued, never counted as discarded bytes
(no epoch was live to discard from). The only trace of them is the "seen" counters.

The native reader outlived the wire. It opened files 29 to 32 at 15:48 to 15:56, and native streams went from 27 at
15:40 to 32 at 16:10 (E/log.jsonl). File 32 by start time is `/root/stage6_integrated_review` (15:56:43). The 11 after
it are every subagent from 16:44 to 16:53, the same ones with no wire record. Those 11 left nothing on the server
(verified on the server copy, section 9.3).

### 1.2 Two causes the report did not name

- **Reconnects stay at 10 s.** `attempts` in the proxy writer never resets (`proxy/ingest.rs:861-878`), so from the
  5th epoch of a launch every reconnect waits 10 s (`RECONNECT_DELAYS`, `:95-100`, schedule 1, 2, 5, 10 s). Each load
  drop cost at least 10 s of capture for every subagent, plus every stream open at the drop.
- **The daemon refuses new epochs under load.** A fresh epoch is answered `Pending` while the daemon queue is over
  half full (`sink.rs:12-13, 316-319`). The receiver polls only until its 5 s setup deadline (daemon
  `ingest.rs:147, 744-760`), then fails the producer `Cancelled` and closes. The proxy reads that as one more
  `IngestLost` and counts it. Under a long burst, the recoveries themselves burn the budget.

### 1.3 Corrections to the report and the handover

| # | The report or handover says | Corrected | Evidence |
|---|---|---|---|
| C1 | 8 MiB "is about five request bodies" | About **one and a half**: the JSON number arrays charge about 3.6 bytes per payload byte, so the lane holds about 2.3 MB. Estimate from a text sample, not the run's bodies; binary or non-ASCII encodes up to 4x. | `capture.rs:228-231`, `sink.rs:222-238` |
| C2 | The queue constant is in `sink.rs` | It is defined in the protocol crate; `sink.rs` derives the payload lane from it | `capture.rs:50-51`, `sink.rs:7-10` |
| C3 | "Ten short epochs in a row" (handover) | The 161 s epoch's end is failure 1; nine short epochs follow. The report's own "the 10th drop in a row" is right. | `proxy/ingest.rs:1117-1133` |
| C4 | "No gap marks the loss" / "the daemon never saw a loss to mark" | **The start is marked**: the 10th drop wrote a `queue_full` gap at 15:43:15 like the 21 before it. What is missing is the end (no `Recovery` ever follows), any mark that this drop was final, and the proxy's disabled state, which the seal skips. Only a reader who looks for a missing `Recovery` finds it. | `owner.rs:326-341`, `writer.rs:182-201` |
| C5 | Codex resends 1.5 MB per call | True for the 243 HTTP POSTs only. WebSocket client messages average about 236 KB. | E/final.json |
| C6 | "Chunks of up to 264 MB" | Those are chain bytes per chunk (2.8 MB to 264 MB raw, `host.log`). A chunk is at most 32 MiB compressed. | `goalcup-cli/src/upload/uploader/chunk.rs:12-16`, `209-214` |
| C7 | "Backoff to 18 min" | The cap is 15 min plus 0 to 25% jitter. The waits kept growing because the failure count resets only when the upload fully catches up, which never happened under load (section 4.7). | `goalcup-cli/src/upload/policy.rs:84-85`, `283-291`; `goalcup-cli/src/upload/uploader/lifecycle.rs:517-519` |
| C8 | Seal `limit_exceeded` gap: "unknown which" limit | Almost certainly the native finish marker: it fits 32 slots for 43 files exactly. The proxy `capture_table_full` counter (`writer.rs:193`) also writes this reason; it needs 129 concurrent intercepted connections, and the run's open streams peaked at 54, so it is unlikely. Not proven: the chain was pruned. | `owner.rs:139-146`, E/log.jsonl |
| C9 | 479,319 records | That is the count at the seal (report measure 6). The observer's last line (16:40:33Z) shows 479,051; section 4 uses that line for its arithmetic. | E/log.jsonl |
| C10 | 22 `queue_full` gaps, 17 `recovery` records | 5 gaps have no recovery. One is the final 15:43:15 gap. The other four are not explained: they could be extra gaps from one drop (a gap per open stream) or a native loss, but native bytes kept growing to 16:10, so probably not native. Unverified. | E/final.json |

## 2. Every silent-loss path in the code today

"Gap start" means a chained record at the time the loss began. "Gap end" means a chained record, or a well-defined
rule, saying when capture of that thing resumed. "Lines" are at HEAD.

| P | Trigger | What is lost | Gap start today | Gap end today | Participant told? | Smallest fix | Lines |
|---|---|---|---|---|---|---|---|
| P1 | Daemon queue full: one frame over the payload lane | Every in-flight stream of that producer, and everything until the next `Recovery` | Yes, a gap per open stream | Yes, the next `Recovery`, if one comes | "has gaps" only | Turn the refused `Bytes` frame into `Gap{QueueFull, lost_bytes}` on its own stream; keep the producer live (ask 2) | `sink.rs:273-282` |
| P2 | Proxy queue full | Same as P1, proxy side; not counted as a failure | Yes (receiver sees EOF without `Close`) | Yes, next `Recovery` | "has gaps" only | Same gap substitution in the proxy queue | `proxy/ingest.rs:513-516` |
| P3 | **Failure budget (this run):** 10 losses in a row, epochs under 60 s, connect failures included | **All wire capture for the rest of the launch** | Only as an ordinary gap that looks recoverable | **No** | No ("Recording works") | Delete the budget (CH6, ask 1) | `proxy/ingest.rs:98-108, 463-472, 938, 965, 1117-1133` |
| P4 | Daemon `Pending` past 5 s: queue over half full when a fresh epoch arrives | The new epoch, and one more counted failure | Indirect (the previous loss) | No, if it feeds P3 | No | Goes away with ask 1 | `sink.rs:316-319`, daemon `ingest.rs:744-760` |
| P5 | Entry-wide open-stream bound: 128 across wire, native (32) and lost-but-not-superseded producers | Daemon refuses the wire `Open` with `LimitExceeded`, loses the producer; the proxy counts it | Yes | Next `Recovery`, but it feeds P3 | No | Raise or remove (CH1); refuse only the one stream, never the producer | `sink.rs:421-428` (`Open`), `:386-388` (connections) |
| P6 | Emitter open-stream bound: 129th open stream on one producer | **All wire capture for the rest of the launch** (`Disabled`, terminal) | **No** (no `Loss` frame is sent) | Only at the seal: the producer never closes, so the daemon cuts its open streams | No | CH4: refuse that one stream with a `Loss` frame, keep the epoch live | `proxy/api.rs:451-453` |
| P7 | Emitter connection bound: 129th tracked connection | Same as P6, terminal | No | At the seal only | No | CH4 | `proxy/api.rs:599-601` (the `CloseGuard` at `proxy/mod.rs:1455-1468` usually catches it first, see P8) |
| P8 | Capture table full: 129th concurrent intercepted connection | That connection, relayed raw | First time only (P10) | No | One seal gap per non-zero counter (`writer.rs:186-201`) | Raise the bound (CH1), drop the dedupe (P10) | `proxy/mod.rs:1381-1392, 1455-1468` |
| P9 | Spent producer IDs: 65,536 in one launch | All wire capture, terminal `LimitExceeded` | No | No | No | At one reconnect per 10 s the bound lasts 7.6 days, just over 168 hours. Let the daemon forget superseded producers, or raise to 1,048,576 and state the map's memory cost (N3 G2, about 20 src). With ask 2, drops become rare. | `proxy/ingest.rs:560-562`, `proxy/api.rs:713-718`, `sink.rs:308-310`, `sink.rs:183`, `capture.rs:63` |
| P10 | `Loss` dedupe: second fail-open tunnel, opaque fallback, table-full or over-limit exchange in one launch | Each later one, with its time | First per reason per launch only | No | One seal gap per reason | Delete the dedupe: one `Loss` per event (about 5 lines removed) | `proxy/api.rs:671-674, 683` (doc `:640-651`) |
| P11 | Local framing error in the proxy sink (sequence break, frame over a limit: a bug) | All wire capture, terminal | No | No | No | `latch_loss` instead of `latch_terminal` at the 4 call sites, so the next boundary recovers | `proxy/ingest.rs:504-511, 552-558, 600-606, 944-948` |
| P12 | Refused `Recovery` frame: the first frame of a fresh epoch is refused | All wire capture, terminal by design | Yes (the previous loss) | No | No | Leave for Friday; needs the CP-A4 orphan rule; rare once asks 1 and 2 land | `proxy/api.rs:774-785` |
| P13 | Owner-only producer loss: redactor refuses a frame, or the redactor build fails | Everything that producer sends afterwards: the owner drops it silently while the sink and receiver keep admitting it | Yes (gaps from `lose`) | **No.** The sink is never told, so the proxy never reconnects | No | In `Owner::lose`, also latch the producer lost in the sink, so the receiver closes and the proxy recovers (about 3 lines) | `owner.rs:285-287, 296-318, 326-341` |
| P14 | Native producer refused: native `Open`/`Bytes` meets a full shared queue or the open-stream bound | **All native capture for the rest of the part.** "There is no wire-style recovery"; "a loss stays latched for the life of the adapter" | Yes (gaps on open native streams) | No | No: status stops counting native files (`owner.rs:22-25, 186-193`) | Native reads only what fits: ask the sink for room before a pass. The file is the buffer, so this is free backpressure | `native_capture.rs:84-91, 970-973, 649-656`; `sink.rs:273-283` |
| P15 | Native slot wait: more than 32 append-only files | Files beyond 32, for the whole part (11 in this run) | No (waiting is not chained) | One stream-less gap at the seal, no file name, no count | No; status counts waiting files as found (`owner.rs:188`) | CH2 (remove the cap) | `native_capture.rs:279, 1555-1558`; `owner.rs:139-146` |
| P16 | Native file cap: over 1,024 tracked files | Every later file | Stream-less gap, at most one per 60 s | No | No | CH2 | `native_capture.rs:280, 1323-1327`; `owner.rs:200-221` |
| P17 | Native finish budget: more than 64 MiB unread at the seal | Tail of each unfinished file | Yes (`ShutdownDeadline` per stream) | n/a (end of part) | No | CH12 | `owner.rs:13, 130-139` |
| P18 | Redactor hold over 16 KiB | Bytes are masked from that point, not lost | n/a: an audit span marks it | n/a | No | None for Friday | `goalcup-daemon/src/redact/stream.rs:1955-1962`, doc `:66-80` |
| P19 | Decoded body over 8 MiB per read | The rest of that body | Yes (stream gap `LimitExceeded`) | Stream end | No | Leave; follows the queue size | `proxy/http.rs:93-97, 1564` |
| P20 | Writer inadmissible record (for example a connection past 128 in the writer) | That record, silently | No | No | No | Unreachable while the sink checks the same bounds; note only | `writer.rs:126-128, 227-271` |
| P21 | Writer I/O failure: any chain write or sync error | Everything after; the chain cannot hold a gap | No (cannot) | No | Yes, `CaptureState::Failed` (`runtime/capture/mod.rs:1309-1311`) | None: loud is correct | `writer.rs:152-155, 205-223` |
| P22 | Disk full: reservation spent | All payload after; closures still written | Yes (`StorageFull`, per stream and once per entry) | No | "has gaps" | Owner: no special handling (CH9 for the torn tail) | `writer.rs:136-145, 274-288` |
| P23 | Recorder accept loop dies 6 times | Harness connections **refused** (breaks point 3) | n/a | n/a | Yes (`problem_recorder_lost`) | CH6 | `proxy/mod.rs:1022, 1079-1088` |
| P24 | Over 1,024 connection tasks | The harness gets a 503 (breaks point 3) | n/a | n/a | Counted | CH6 | `proxy/mod.rs:112, 121` |
| P25 | Uploader: network errors, 413 at the server's 64 GiB run limit | Nothing local; it waits on disk. A full busy 168 h Codex run is about 68 GB compressed, right at the 64 GiB (68.7 GB) limit, which the uploader retries for ever (`policy.rs:251`) | n/a | n/a | Yes (upload lines) | Not a loss path. Remove the run limit (owner, Q2/Q3; section 4.8) | `goalcup-cli/src/upload/policy.rs:4-7, 80-89`; `server/worker/src/context.ts:12` |

Notes:
- P3, P6, P7, P9, P11 and P12 end the same way as this run: wire capture off for good, no `Recovery`, the finish says
  "works". Fixing only the failure budget leaves the other five.
- P13 and P14 are the daemon-side twins: a producer that stays "live" to the sink, or a native producer latched for
  life. E/final.json shows `producer_closed: 2`, which suggests the native producer closed cleanly here, so P14
  probably did not fire. Unverified.
- P14 is a real risk even if unseen: the owner polls native right after a drain (`owner.rs:161-176`), but a drain
  takes at most 256 items (`DRAIN_BATCH`, `owner.rs:30`), so under a wire flood the queue can still be near full when
  native submits.

## 3. Answers to the handover asks 1 to 6

### Ask 1. A drop under load never counts toward "the daemon is dead"

- **Change:** delete the failure budget (CH6's third part). Remove `MAX_CONNECTION_FAILURES` and `STABLE_EPOCH`
  (`proxy/ingest.rs:98-108`), `consecutive_failures`, `epoch_since`, `epoch_wrote` (`:380-383`), `count_failure`
  (`:461-472`) and its three callers (`:938, :963-970, :1117-1133`). Keep `stats.failures` as a plain log counter.
  The writer retries for ever on the existing 1, 2, 5, 10 s schedule (`:88-93`). Log once a minute, not every attempt.
- **Size:** about 35 src lines removed. Tests rewritten: `remote_eof_does_not_invent_rejection_reason`,
  `ready_then_remote_eof_cannot_reset_retry_budget`, `an_epoch_that_takes_one_item_and_closes_at_once_still_runs_out_at_ten`,
  `a_successful_connection_resets_the_failure_count`, `recovery_budget_exhaustion_stays_incomplete`, and the constant
  check at `m2_capture_sender.rs:1915` (tests start at lines 1008, 1068, 1123, 1161 and 1241, through about 1289).
- **Not before Friday:** resetting `attempts` after a live epoch (1 line, `:861-878`). A daemon that accepts and
  closes at once would then spend one producer ID a second and reach P9 in about 18 hours. At 10 s it takes 7.6 days.
- **Alone, this would have kept the run recording.** After 15:43 the proxy would have reconnected within about 10 s
  each time.
- **Risk:** a daemon that refuses the credential is retried every 10 s for ever: log noise and one ID per 10 s.
- **Proof test:** a sender test with the emulator policy `CloseAfterHandshake` (or "read 3 items then close"), short
  delays (`with_reconnect_delays`), 50 losses in a row each under a second. Assert the sink never answers `Disabled`,
  `stats.epochs_ready >= 50`, and a frame submitted after the 50th loss is `Queued`.

### Ask 2. Backpressure or a bigger buffer per stream, not dropping a whole producer

- **Daemon side (the side that dropped this run's producer):** in `OwnerSink::try_submit` (`sink.rs:268-282`), when a
  `Bytes` frame does not fit, queue a replacement instead of losing the producer: same stream, same `seq`, same
  `observed`, event `Gap { reason: QueueFull, lost_bytes: Some(data.len()) }`, charged its real small cost. Answer
  `Queued`. Everything downstream already accepts it:
  - the sink admits a later `Bytes` at any offset at or past the stream's next offset after a gap (daemon
    `ingest.rs:1002-1008`, `sink.rs:539-546`);
  - the redactor closes the segment, moves `next_offset` by `lost_bytes` and resumes at the offset the proxy sends
    next (`redact/stream.rs:2834-2861`);
  - the chain record carries the stream, the time and the byte count (`capture.rs:295-300`).
  Non-`Bytes` frames (`Open`, `End`, `Gap`, stream-zero controls) are small: they use the control reserve, raised to
  hold a few thousand. Only if even a small frame cannot fit does today's producer loss remain, and with ask 1 it
  always recovers.
- **Proxy side:** the same in `State::submit` (`proxy/ingest.rs:513-516`). The frame is encoded before the lock, so
  `try_submit` (`:804-817`) must keep it to build the gap. About 15 lines.
- **Cheap multiplier:** serialize `FrameEvent::Bytes.data` as base64 (`capture.rs:228-231`, one serde attribute;
  regenerate `goalcup-protocol/tests/fixtures/m2-ingest/golden-stream.hex`). The same 8 MiB then holds about 6 MB of
  payload, not 2.3 MB, and the daemon parses a quarter of the bytes. It also cuts relay CPU: today each 64 KiB frame
  becomes about 240 KB of JSON numbers under the shared emitter lock that every subagent's relay takes in turn
  (`proxy/api.rs:497-561`, `proxy/ingest.rs:804-817`); base64 cuts that work about 2.7 times (latency not measured).
  No backward compatibility is needed before Edition 01.
- **Size:** about 40 src over both sides, 1 protocol line, one fixture.
- **Estimate conflict:** N3 (G5) put a per-stream refusal at about 200 src and 300 tests, after Edition 01, assuming
  new per-stream "lost" state. M1's reading shows the gap substitution needs no new state, because the gap moves the
  stream's offset. This appendix takes the smaller form for Saturday (section 6.5), and keeps G5 as the fallback if
  the 40-line form fails review.
- **Risk:** a request body can now have holes: that stream is "incomplete", not invalid, and every other stream keeps
  recording. The verifier's provider decoders meet `Gap` records already, but a mid-body gap in `openai.rs` is not
  tested (unverified). Fairness: with one shared queue, the stream that submits when the queue is full loses bytes,
  maybe a small WebSocket message rather than the 1.5 MB body that filled it. A per-stream share (refuse a stream's
  frame when it alone holds over about an eighth of the queue) fixes that for about 10 more lines; optional.
- **Proof test:** invert `full_queue_gap_is_sticky` (`runtime/capture/tests.rs:1061`) and
  `queue_full_reports_loss_outside_full_queue` (`capture_ingest.rs:1668`): fill the sink, submit `Bytes`, assert
  `Queued`, the producer is live, the drained item is the `Gap` with `lost_bytes == len`, the next `Bytes` at
  `offset + len` is accepted, and the sealed trace verifies "incomplete" with exactly one stream gap. Same shape for
  the proxy in `queue_full_latches_without_blocking_relay` (`m2_capture_sender.rs:297`).

Why not true backpressure on the wire. The capture path is synchronous inside the relay by design: the emitter holds
its lock through the sink's non-blocking `try_*` call and never waits (`proxy/api.rs:5-8`, `proxy/ingest.rs:19-27`).
Making the relay wait for our writer is what point 3 forbids: the harness would pause whenever the owner thread is
slow, the disk syncs or the laptop wakes, and hang if the daemon is stuck.

| Option | Slows the harness? | Loses | Code | Fits Friday? |
|---|---|---|---|---|
| A. Bigger shared queue (CH1, 64 MiB; plus base64) | No | Still a whole producer when exceeded | 1 to 3 lines | Yes, as a frequency cut, not a fix. 64 MiB with base64 holds about 48 MB of payload, about 30 bodies. Worst-case memory 64 MiB per side. |
| B. Per-stream gap on overflow (ask 2) | No | Only the overflowing bytes, chained per stream with a count | about 40 lines | **Yes. Recommended**, with A |
| C. Per-stream share inside B | No | Only the stream that hogs the queue | about 10 more | Optional; fairer |
| D. Spill to disk when full | No, unless the disk stalls | Nothing | about 150 to 300 lines: an ordered spill file read back in admission order across producers, cleanup, crash handling | No. Only if a measurement shows sustained inflow above the owner's throughput. Average payload while recording was about 0.25 MB/s. |
| E. The proxy writes the record itself (the owner's "local proxy" idea) | No, with its own queue | Nothing between proxy and daemon: that hop is gone | Large: redaction, chain writer and run key move into the proxy process | No for Friday. The right long-term shape: it removes the IPC hop, both 8 MiB queues, the JSON byte arrays and the epoch and recovery protocol (`proxy/ingest.rs`, daemon `ingest.rs`, most of `sink.rs`). Same as plan K13. |
| F. Native backpressure (P14 fix) | No: the harness writes its files, we read later | Nothing | about 10 lines | **Yes.** The file is the buffer |

Would round-1 CH1 and CH6 alone have saved the run?

| Item | Would it have prevented the permanent loss at 15:43? | What it misses |
|---|---|---|
| CH1 (queues 8 to 64 MiB, streams 128 to 16,384) | **Probably not on its own.** With the 3.6x JSON encoding, 64 MiB holds about 18.6 MB of payload, about 12 HTTP bodies. A synchronized retry of 20 subagents after a rate-limit window is about 30 MB, of 43 subagents about 65 MB. Drops get rarer, not gone, and the budget still ends capture after ten. N3's "64 MiB holds about 40 bodies" used the report's uncorrected figure (C1). Unverified: peak queue depth and owner throughput were not measured. | The cliff (P1, P2), the JSON encoding, memory (64 MiB each side worst case) |
| CH6 (no restart cap, no 503, no `MAX_CONNECTION_FAILURES`, retry every 10 s) | **Yes for "off for good".** | Each drop still costs every in-flight stream and at least 10 s. At this run's rate (17 drops in 28 min) that is over 3 minutes blind plus every cut response, in bounded gaps. It does not touch P6, P7, P9, P11, P12, P13, P14, the finish count, or the off-time display. |

Together CH1 and CH6 turn 71 silent minutes into many short chained gaps. Asks 2 to 4 are still needed.

### Ask 3. Loss can never be silent: every loss has a start and an end

Define the end once, with no new record: an off span of a producer starts at its chained loss gap and ends at the
next `Recovery` record, or at the part's `LocalFinish` if none comes. The verifier and the finish can compute that.
Then close every path with no start or no possible end:

| # | Path | Change | Size |
|---|---|---|---|
| 1 | P3 | Ask 1 | about -35 src |
| 2 | P6, P7 | CH4: refuse the one stream with a `Loss` frame, keep the epoch live | about 4 lines |
| 3 | P11 | `latch_loss` instead of `latch_terminal` at 4 call sites. P12 stays terminal for Friday | about 4 lines |
| 4 | P10 | Delete the per-emitter `Loss` dedupe (`proxy/api.rs:671-674, 683`) | about -5 lines |
| 5 | P13 | In `Owner::lose`, also latch the producer lost in the sink (`owner.rs:326-341`) | about 3 lines |
| 6 | P14 | Give `OwnerSink` a `room()` read; cap each native pass at it (`native_capture.rs:649-656`). The native reader position moves only on `Queued` (`native_capture.rs:964-968`), so unread bytes stay in the file | about 10 to 20 src, 1 to 2 tests |
| 7 | P15, P16 | CH2 (no native caps) | about -40 src |
| 8 | Seal | Write one stream-less `IngestLost` gap when `sink_refusals > 0` (`writer.rs:189-199`). Better: the proxy reports when its sink went down and came back, and the seal writes a gap with that start and end (N3 G3) | about 5 src (first form), about 40 (second) |
| 9 | P9 | Forget superseded producers or raise the bound (N3 G2) | about 20 src |

Risk: item 4 adds one small record per pass-through connection to an approved host: a few records per session. Item 6
means native capture can fall behind the wire during a storm; it catches up from the file, and the finish drains it
(CH12 removes the 64 MiB finish cap). Item 8 can give one loss two gaps (one chained, one at the seal); gap counts
count records, not losses, as `control.rs:688-689` already says.

Proof tests: one integration test per path that today ends `Disabled`: open 200 streams at once (P6), force a
redactor refusal (P13), fill the queue while native has a 10 MB file to read (P14). For each, assert a gap at the loss
time and a `Recovery` (or, for native, the full file bytes) afterwards, and that no relay call waited.

### Ask 4. Count a call only once its bytes are in the chain; show how long recording was off

- **Recorded calls, counted by the daemon writer.** In `ChainWriter::note_written` (`writer.rs:327-345`), count each
  written `Open` whose source is `WireRequest` with method `POST`, or `WsClient`, and that carries a `protocol_hint`
  (WebSocket message streams carry the connection's hint, `proxy/websocket.rs:304-318`). Expose it in the signed
  status (`runtime/capture/mod.rs:1292-1301`) and the finish report, and print it instead of the proxy's selection
  count (`finish.rs:405-407, 439-444`). The proxy's count stays as "seen": "Recorded 842 model calls of 1,736 seen".
  WebSocket interrupts still count as messages (37 here): say "messages" for WebSocket, or accept the small
  overcount, or let connector data name the message type that starts a generation (data, not a Rust literal,
  `AGENTS.md:87`). About 25 src and a `CONTROL_API_VERSION` bump.
- **Off time, kept by the emitter.** The emitter is the one place that decides capture on or off (`Epoch` at
  `proxy/api.rs:158-171`). Route every change of `state.epoch` through one helper that notes when it left `Live` and
  adds the span when it returns (`try_recover` at `proxy/api.rs:739-745`). Publish total off time, number of spans
  and "off since" as shared atomics like `model_requests_counter` (`proxy/api.rs:351-353`). The finish and
  `goalcup status` print "Recording was off 17 times, 1 h 12 min in total" or "Recording is off since 15:43".
  About 20 src.
- **Plan impact:** AD2 as written reads `model_calls` (`goalcup-cli/src/run/mod.rs:266, 716-719`), the selection
  counter that printed 1,736. It must read the written count instead (N3 G4, about 60 src and 60 tests with the
  status line and wording). Show only the written count to the participant. Move the support block
  (`proxy admission: accepted=891 intercepted=772 tunneled_not_approved=118 auth_refused=1`, "anchor", "terminal
  record", "seal.json binds it", the dead `verify:` line; E/final-pane.txt) to `host.log` only.
- **Risk:** "written" is not "synced": a crash can still lose the last 800 ms (`owner.rs:10`). Good enough for a
  participant line. (Corrected in 9.6: the server keeps no count from the chain; the Worker never opens a chunk.) The heartbeat's `model_requests` (CH14 keeps it)
  stays a "seen" count; say so in the Worker comment.
- **Proof test:** the sender against a daemon whose sink drops one producer mid-run; assert the finish count is
  below the seen count, equals the verifier's `wire_model_requests` on the same trace
  (`goalcup-verifier/src/capture_analysis/session.rs:210`), and the off time is above zero.

### Ask 5. No native stream cap

Yes, but not by deleting the two numbers alone: see section 5. Three things move together: the native caps (CH2),
the shared 128 open-stream bound (CH1 or a native exemption) and the latch on refusal (P14 fix). A cheap poll (CH3)
should land before or with them, because the poll runs on the chain writer's thread.

### Ask 6. Trace size: one stream per WebSocket direction, dedupe, or accept and plan

For Friday: accept the size, add long-window zstd (2 lines), fix the upload backoff (3 lines plus 1), remove the
per-run limit by config. After Edition 01: one stream per WebSocket server direction (or before, if the second run
shows the writer is the bottleneck, because it cuts records per second by about two thirds). Prefix dedupe gives no
more than a long window and costs hundreds of risky lines: not before Edition 01. Details in section 4.

## 4. Trace size and upload

### 4.1 One WebSocket server message is one stream of three records

| What | Where |
|---|---|
| The design says it: "Every data message ... is one stream of source `WsClient` or `WsServer`" | `proxy/websocket.rs:10-23` |
| Each new data frame that is not a continuation calls `begin` | `proxy/websocket.rs:665-667` |
| `begin` opens a stream under the shared index lock | `proxy/websocket.rs:691-716`; `open` `950-962`; `open_at` `965-974` |
| The stream lives inside `Message`, not the direction | `proxy/websocket.rs:559-570` |
| `frame_end` emits `End(Complete)` at the message's FIN | `proxy/websocket.rs:749-774` |
| Metadata per message: `wire.message_index`, label `websocket server text message` | `proxy/websocket.rs:303-320` |
| Model-call counting at selection, every client message start, interrupts too | `proxy/websocket.rs:492-507` |
| The verifier reads each WS stream as one JSON document | `goalcup-verifier/src/capture_analysis/tracker.rs:152-154` (`Framing::Json`) |
| The verifier counts one model request per `WsClient` Open | `goalcup-verifier/src/capture_analysis/operations.rs:174-189` |

Run numbers (observer's last line, E/log.jsonl, 16:40:33Z): `open:ws_server` 152,728, `seg_bytes:ws_server`
103,508,756, `open:ws_client` 671, `records` 479,051, `end_reasons.complete` 154,824. So 99.6% of WebSocket streams are
server messages, and their Open, Segment and End records are at least 458,184 records (95.6%). The average server
message is 678 bytes of payload in about 2,900 bytes of chain.

What one stream per direction takes:

| Piece | Change | Size (est.) |
|---|---|---|
| Proxy | Move `stream: Option<Stream>` from `Message` to `Capture`. `begin` opens only when the direction has none; `frame_end` no longer ends it; the relay end (`interrupt`, `Drop`, `proxy/websocket.rs:985-1013`) does. A refused `flush` (`849-891`) drops it; the next message reopens after `try_recover`, as today | about 30 to 50 src touched, net about -10 |
| Message boundaries | OpenAI server events are JSON objects, so concatenated values split with a streaming JSON reader (`serde_json` `StreamDeserializer`), no added byte | see verifier |
| Verifier | `tracker.rs:152-154`: a new `Framing::JsonSeq` for `WsServer`, each value to `P::json`, `MAX_EVENT_BYTES` per value | about 50 to 80 src |
| Counting | Client messages stay one stream each, so `is_model_request` (`operations.rs:181-184`) and request decoding do not change | 0 |
| Tests | `goalcup-cli/tests/it/m2_websocket_relay.rs` (2,074 test lines) asserts per-message streams; parts of `m2_openai_decoder.rs` (2,149), `m2_run_stats.rs` (873), `m2_proxy_integration.rs` (3,025) | about 300 to 600 test lines changed |

Smallest safe scope: **server direction only, OpenAI hint only.** Client messages (671) carry the model calls; keeping
them per message keeps ask 4 simple. The Amp thread decoder reads `WsServer` and handles non-JSON text such as `ping`
and `pong` (`goalcup-verifier/src/capture_analysis/thread.rs:48-51, 115-118`); concatenated, a `ping` would break the
split. Saving: at least the Open and End records of 152,728 messages, about 305,000 records and 218 MB (17% of the
chain). Segments then hold one 200 ms flush (`FLUSH_EVERY`, `owner.rs:7`, `242-249`), at about 52 messages a second,
so segments fall too. Estimate: 300 to 320 MB saved, about a quarter of the chain. Not verified on real data.

Synthetic check (20,000 messages of about 760 bytes):

| Shape | Chain bytes | Chain / payload | zstd -3 | zstd -19 |
|---|---|---|---|---|
| One stream per message (today) | 61.7 MB | 4.0x | 10.47 MB | 8.78 MB |
| One stream per direction, a segment per 10 messages | 22.0 MB | 1.44x | 4.78 MB | 3.90 MB |

What could go wrong: a server message that is not valid JSON breaks the split for the rest of the stream (the bytes
stay in the chain; the verifier must report `malformed`, never drop silently). A direction stream lives as long as
the connection and holds an open-stream slot (`capture.rs:58`, `sink.rs:425-427`): with 64 or more subagents on
WebSocket at once that cap is hit, so this lands only after the stream caps go. The redactor sees messages joined: a
secret split over two messages is now caught, and a held candidate can hold bytes across a boundary
(`MAX_HELD_BYTES` still bounds it).

### 4.2 Why the chain is 1.81 times the payload

Record format: one JSON line per record with `protocol_version`, `entry_id`, `source`, `recorded_at`, `seq`, `prev`,
`hash`, `body` (`goalcup-protocol/src/envelope.rs:9-28`). `prev` and `hash` are 64 hex characters each,
`hash = hex(sha256(prev_hex || bytes))` (`goalcup-protocol/src/chain.rs:5, 31-36`). Payload is base64 in a `Segment`
(`data_b64`, `capture.rs:280-289`), encoded after redaction (`redact/stream.rs:2427-2437`). Each `Segment` also carries
the 32-hex producer key, `part`, `source_offset`, `source_len`, two `CaptureTime`s (wall, `mono_ns`, 32-hex
`clock_id`) and a redaction audit. A segment holds at most 64 KiB raw (`capture.rs:43`) in a 124 KiB record budget
(`redact/stream.rs:139-140`); every open segment closes every 200 ms (`owner.rs:7, 366-381`). A short fixture record is
516 to 764 bytes (`goalcup-verifier/tests/fixtures/v5-corpus/sealed-key-only/chain.jsonl`).

| Part | MB | How |
|---|---|---|
| Payload | 720.8 | wire_request 375.4, ws_client 158.2, ws_server 103.5, native 45.8, wire_response 36.4, other 1.5 |
| Base64 growth | +240.3 | payload / 3 |
| Record framing | +342.4 | 1,303.5 - 961.1, about 715 bytes per record over 479,051 records |
| Chain | 1,303.5 | 1.81x payload |

About 328 MB of the 342 MB framing (458,184 records x 715 bytes) is the per-message WebSocket server streams. Without
them the chain would be about 1.36x the payload, almost all base64.

Base64 also hurts compression. Segments are cut where the 200 ms flush falls, so the same bytes in the next request
start at a different place mod 3, and base64 of the same bytes in a different phase is different text (rows B2 and C3
below).

| Option | Effect | Lines | Risk |
|---|---|---|---|
| Per-direction WS server streams (4.1) | -17% to -25% chain | about 50 to 130 src, 300 to 600 tests | Medium |
| Cut segments on a multiple of 3 bytes of stream offset | No size change; long-window ratio 14.5x to 37x | about 10 to 20 src in `redact/stream.rs` (`place`, `flush`), tests in `capture_redaction.rs` (4,745) | Medium: "output never depends on chunking" (`redact/stream.rs:62-63`) must stay true |
| UTF-8 text instead of base64 when valid | -20% chain, no phase problem | about 60 to 120 src across protocol, daemon, verifier | High: after Edition 01 |

### 4.3 JSON number arrays over IPC

Separate from the chain format: the hop from proxy to daemon sends payload as JSON arrays of numbers (C1). That
inflates every byte about 3.6 times in both 8 MiB queues and in the daemon's parser. One serde attribute fixes it
(ask 2). It does not change the chain or the upload.

### 4.4 Codec and chunk facts

| Side | Fact | Where |
|---|---|---|
| Uploader | One zstd frame per chunk, `zstd::stream::Encoder::new(tally, ZSTD_LEVEL)` | `goalcup-cli/src/upload/uploader/chunk.rs:144` |
| Uploader | Level 3, "No long window"; the comment assumes about 10x | `chunk.rs:17-19` |
| Uploader | At most 32 MiB compressed per chunk; stops taking lines at 30 MiB; no cap on raw bytes (2.8 MB to 264 MB of chain per chunk in `host.log`) | `chunk.rs:12-16`, `209-214`, `141-196` |
| Uploader | A chunk every 30 min, or as soon as 256 MiB of chain waits; poll 30 s | `goalcup-cli/src/upload/policy.rs:80-83` |
| Uploader | PUT timeout 600 s; backoff 60 s doubling to 15 min, plus 0 to 25% jitter | `policy.rs:14, 84-85, 283-291` |
| Uploader | Heartbeat every 300 s, 30 s timeout | `policy.rs:88`, `goalcup-cli/src/upload/uploader/heartbeat.rs:74` |
| Worker | `Content-Type: application/zstd` only; at most 33,554,432 bytes compressed; `Goalcup-Length` (raw) has no cap | `server/worker/src/chunks.ts:22, 160-170, 177-182` |
| Worker | Streams the body to R2 create-only, R2 checks the SHA-256, never decompresses | `chunks.ts:199-227` |
| Worker | Key `traces/{event}/{run}/{entry}/{offset:016}-{sha256}.zst` | `chunks.ts:155-158` |
| Rebuild | `zstd -dcq` per chunk | `scripts/fetch-run.sh:327` |

Level 3 on large input uses a 2 MiB window (measured about 7 MiB memory). `zstd -d` accepts windows up to 2^27
(128 MiB) with no flag, so `--long=27` output needs no `fetch-run.sh` change (from zstd's documented default; not run
against `fetch-run.sh`).

### 4.5 zstd long-window experiment

Synthetic text only: a Zipf draw over 6,000 made-up words plus code-like lines; a body is
`{"model","input":[...],"tools":[...],"stream","prompt_cache_key"}`, and each request adds one 30 KB item to `input`.
Tools come after input, so a body is not a pure prefix of the next. "Chain-shaped" wraps bytes in records like the real
ones (Open, base64 Segments, End, a real SHA-256 link). "Timed cuts" cut segments at random 4 to 64 KiB, like the
200 ms flush. Python `zstandard` 0.25 (no zstd CLI here); `--long=27` means `window_log=27` plus long-distance matching.
Script and raw output: `/tmp/claude-0/-home-user/c3485515-d491-51f1-8b7f-6278b4ef8e37/scratchpad/m2exp/exp.py`,
`single.txt`, `inter.txt`, `c3.txt` in the same folder (scratch, not in the repo).

**A. One agent, 50 bodies, last body 1.56 MB, total 40.8 MB**

| Data | gzip -9 | zstd -3 (today) | zstd -3 --long=27 | zstd -19 | zstd -19 --long=27 |
|---|---|---|---|---|---|
| A. plain JSON | 3.0x | 74.8x | 77.8x | 80.8x | 89.6x |
| A'. prefix and suffix dedupe first (1.6 MB left) | 3.0x of the rest | 3.3x of the rest (0.47 MB) | | 3.9x of the rest (0.40 MB) | |
| B1. chain-shaped, 64 KiB aligned cuts (55.0 MB) | | 12.7x | 71.6x | | 83.1x |
| B2. chain-shaped, timed cuts (55.4 MB) | | 5.3x | 28.5x | | 34.8x |

**C. 20 agents x 25 bodies, interleaved at random, 218.4 MB** (close to the run's about 20 subagents)

| Data | zstd -3 (today) | zstd -3 --long=27 | zstd -9 --long=27 | zstd -19 --long=27 |
|---|---|---|---|---|
| C. plain JSON | 4.2x (51.9 MB) | 42.8x (5.10 MB) | 45.5x | 51.9x (4.21 MB, 14.5 s) |
| C'. prefix and suffix dedupe per agent (16.0 MB left) | 3.5x (4.64 MB) | | | 4.2x at -19 (3.81 MB) |
| C2. chain-shaped, timed cuts (296.7 MB) | 4.0x (74.3 MB) | 14.5x (20.4 MB) | 15.9x | |
| C3. chain-shaped, cuts on a multiple of 3 bytes | 4.3x | 37.2x (7.97 MB) | 40.5x | |
| C2 in 30 MiB chunks like the uploader | 4.0x (3 chunks) | 14.5x (1 chunk) | | |

Memory to compress (peak RSS growth, streaming 300 MiB): level 3 default 7 MiB; `window_log=26` with LDM 73 MiB;
`window_log=27` with LDM 141 MiB; level 9 with `window_log=27` 159 MiB.

What it means:
- gzip's 32 KB window never sees the earlier copy: only the text's own 3x.
- Level 3 works for one agent (A: 74.8x) because the previous body fits in 2 MiB. Add base64 and records (B1: 12.7x)
  or 20 interleaved agents (C: 4.2x) and the copy is out of the window. That matches the run: the first big chunk (one
  agent warming up) went from 264 MB to 30.4 MB (8.7x); later chunks 2.3x to 3.5x (`host.log`); the whole run 3.2x.
- `--long=27` restores most of it: chain-shaped data 4.0x to 14.5x (3.6 times smaller), plain JSON 42.8x.
- The base64 phase costs a lot under a long window: 14.5x with timed cuts against 37.2x with cuts on a multiple of 3.
- Prefix dedupe is no better than a long window: C' 4.64 MB against C 5.10 MB at level 3. It would help only the local
  disk and the daemon queue.
- Synthetic text is not real context. The direction and size of the gap are the finding, not the exact numbers.

**168 hours at 1.3 GB an hour raw** (the busiest recorded rate, about 20 subagents):

| Scenario | Raw per 168 h | Compressed per 168 h | Upload rate needed |
|---|---|---|---|
| Today (3.2x, measured) | about 218 GB | about 68 GB (63 GiB), at the 64 GiB per-run limit | about 0.9 Mbit/s all week |
| Long window only (est. 2x to 3.6x better) | about 218 GB | about 19 to 34 GB | about 0.25 to 0.45 Mbit/s |
| Long window plus per-direction WS server streams (est.) | about 165 GB | about 15 to 27 GB | about 0.2 to 0.35 Mbit/s |

These are worst cases from the busiest hour with rate-limit retries; a sustained week is not measured. The earlier
busy Hermes test measured 600 MB an hour raw and 5 to 7 GB compressed per 168 h
(`docs/research/results/2026-10-22-local-test-day.md:133`, via appendix I).

### 4.6 Dedupe options

| Option | What | Lines | Helps | Risk |
|---|---|---|---|---|
| 1. Long-window zstd | In `chunk.rs:144`, after `Encoder::new`, set `window_log(27)` and `long_distance_matching(true)`; fix the comment at `chunk.rs:17-18` | about 2 to 4 src, 1 test | Upload bytes and R2 cost, 3x or more | Low. `goalcup start` grows about 140 MiB while a chunk is built (it was 26 to 32 MB RSS in the run); use `window_log(26)` (73 MiB) on small machines. The `zstd` 0.13 crate builds without default features (`Cargo.toml:72`); that the two setters exist there is unverified until it builds. No Worker, rebuild or verifier change. No help for local disk or the daemon queue. |
| 2. Prefix dedupe in proxy or daemon | Store a body as "stream X up to offset N" plus the new part | about 300 to 600 src, 500 or more tests (est.) | Upload, R2, disk, queue | High. New record kind across protocol, daemon, redactor, verifier, rebuild. A reference must point at redacted bytes; bodies are not pure prefixes; a lost referenced stream makes later ones unreadable. Not for Friday. |
| 3. Accept the size | Nothing | 0 | nothing | About 68 GB compressed per busy week; 0.9 Mbit/s all week; a slow uplink finishes with `goalcup upload` later. The per-run limit must go. |
| 4. Base64 phase fix (4.2) | Cuts on a multiple of 3 | about 10 to 20 src | With option 1: 14.5x to 37x | Medium (redactor). After option 1, with a test that segment bytes never depend on chunking |

Recommendation: option 1 now, option 3 for the rest, per-direction WS server streams as the one format change worth
doing (it also cuts writer load), options 2 and 4 after Edition 01.

### 4.7 Upload lag and the backoff fix

What happened (`host.log`): 20 chunks, 11 network errors (os errors 11, 104, 32, one EOF), waits of 61, 130, 269, 497,
987, 975, 1,083 and 947 s. Those add up to 4,949 s, about 82 of the run's 120 minutes. At t+61 min only 94 MB of the
405 MB compressed had been sent (7%); at 16:40, 276 MB. All 20 chunks, 1,304,968,017 bytes of chain, were acknowledged
with the seal by 16:54:46Z, 3 s after the seal.

Why the waits grew: `failures` goes up on every failed attempt (`lifecycle.rs:529-531`) and is set to 0 only on
`Attempt::CaughtUp` (`lifecycle.rs:517-519`). While the agent records faster than the upload sends, it never catches
up, so the count kept doubling across chunks that were stored in between: 61, 130, then 269 after a stored chunk, then
497, up to the 15-minute cap plus jitter (`policy.rs:85, 283-291`).

Why the errors happened: not known. Heartbeats (30 s timeout, tiny body) also failed 3 times in the same window, so the
network path (sandbox egress or Cloudflare) is more likely than chunk size. `os error 11` from `ureq` usually means a
socket timeout. Both unverified.

| Change | Where | Lines | Effect | Risk |
|---|---|---|---|---|
| Reset `failures` whenever an attempt moved the server's offset (compare `up.pos.offset` before and after `attempt`) | `lifecycle.rs:513-533` | about 3 | The run's waits would have been about 60 to 130 s, not up to 18 min | Low |
| `backoff_max` 15 min to 2 min | `policy.rs:85` | 1 | Lag after a burst of errors is a few minutes | Low. A retry resends at most 32 MiB; the Worker answers a resent stored chunk without reading the body (`chunks.ts:44-47`). The upload never blocks the harness |
| N3 G7 adds: `eager_bytes` 256 MiB to about 32 MiB of chain, interval 30 to 5 min | `policy.rs:80-90` | 2 | Server copy stays close behind | More D1 rows and R2 objects (about 2,000 to 10,000 a week) |
| Optional: `CHUNK_SOFT_BYTES` 30 MiB to 8 or 16 MiB | `chunk.rs:16` | 1 | Less resent after an error | More Class A operations and `chunks` rows (about 8,000 per busy week at 8 MiB). Only if errors grow with body size |
| Heartbeats | none | 0 | A time anchor, not data; three misses lose nothing | none |

With long-window compression the same chain needs about a third of the bytes, so lag shrinks too.

### 4.8 Storage cost without a per-run limit

Facts: R2 Standard storage is $0.015 per GB-month; 10 GB-month, 1 million Class A and 10 million Class B operations
free each month; Class A $4.50 per million; no egress fee; no minimum storage duration on Standard
([R2 pricing](https://developers.cloudflare.com/r2/pricing/)). The production bucket has a 180-day bucket lock on
`traces/goalcup-cup-2026/` (`server/worker/wrangler.toml:29-32`, `docs/SERVER-DESIGN.md:84-85`); locked objects cannot
be deleted or overwritten until it ends, though a rule can be removed
([R2 bucket locks](https://developers.cloudflare.com/r2/buckets/bucket-locks/)). "No lifecycle rule deletes objects
yet" (`docs/SERVER-DESIGN.md:85`). Today's limit: `RUN_MAX_COMPRESSED_BYTES = 68719476736` (64 GiB)
(`server/worker/wrangler.toml:56-59`, default `server/worker/src/context.ts:12`, checked `chunks.ts:117-142`); 0 is
refused (`context.ts:61-66`).

Kept at least 180 days, worst case every entry runs a busy 168 hours:

| Per run, compressed | Per month | Per run, 180 days | 100 entries | 500 entries |
|---|---|---|---|---|
| 68 GB (today's rate) | $1.02 | $6.12 | $612 | $3,060 |
| 27 GB (long window, low end) | $0.41 | $2.43 | $243 | $1,215 |
| 15 GB (long window plus per-direction WS) | $0.23 | $1.35 | $135 | $675 |
| 6 GB (the busy Hermes rate) | $0.09 | $0.54 | $54 | $270 |

Operations: at 30 MiB chunks a 68 GB run is about 2,200 PUTs; 500 runs about 1.1 million, about $0.50 over the free
tier. What the limit guards: a stuck or hostile client. At 10 MB/s nonstop that is about 6 TB a week, about $90 a month
and $540 for 180 days under the lock, per run.

- **Remove for Friday, config only:** set `RUN_MAX_COMPRESSED_BYTES` to `9007199254740991` (the largest value
  `parseSafeUint` takes) in `server/worker/wrangler.toml:59, 151`. Zero code. Owner answer Q2/Q3.
- **Or an abuse guard that says so:** 1 TiB (about $15 a month per run), 15 times the worst measured week. The
  `quota-half` and `quota-full` log lines (`chunks.ts:137-145`) warn first.
- **Or cut the code** (about 26 src in `chunks.ts`, 114 test lines in `quota.test.ts`, appendix I): after the event.

Blind spot: the 24-hour close after the run end (`server/worker/src/db.ts:17`) is also our own limit. 68 GB at
5 Mbit/s is about 30 hours. Long-window compression mostly removes this.

### 4.9 Pruning at the seal

During the run a part is deleted 2 hours after the server acked it, or at once under disk pressure, never the last
(`goalcup-cli/src/upload/prune.rs:1-31`, `deletable` `47-81`). At the end, `clean_finished_runs` deletes every part
but the last of each sealed, fully sent entry (`goalcup-cli/src/commands/start/finish.rs:288-290`, `prune.rs:115-171`);
`goalcup upload` and `goalcup status` call it too (`commands/upload.rs:136, 158, 201`; `commands/status.rs:104`). The
seal still prints `verify: goalcup-verify --capture-integrity <trace>` (`commands/seal.rs:452`), which fails with
`chain_pruned_locally` (E/verify-local.txt). So the full 1.3 GB was never scanned for secrets (E/secrets-scan.txt
covers only the 60 files left: the last 1.4 MB part, 43 rollouts of 58.5 MB, tunnels, logs, upload state).

Recommendation: keep prune on ack (owner Q9, plan CH10), drop the 2-hour timer and disk branch (about -60 src). Fix the
message (about 5 src in `seal.rs`): print `verify:` only when the local chain is whole, otherwise "The server holds the
full trace; this machine keeps the last part, the seal and the acknowledgements." Organizer runs keep their own copy
(section 7). What could go wrong: a server bug that loses a stored chunk can no longer be repaired from the
participant's machine. Low likelihood (create-only puts, SHA-256 checked, hash in the key), but real.

## 5. Native caps and the local-logs reconcile

### 5.1 The caps

| Limit | Path:line | Value | What it guards today |
|---|---|---|---|
| Native open streams | `native_capture.rs:279`, checked `:1555-1558` | 32 | Leaves 96 of the entry's 128 to wire capture |
| Tracked files | `native_capture.rs:280`, checked `:1323-1327` | 1,024 | Map memory and poll CPU |
| Hard ceiling on tracked files | `native_capture.rs:187` (`MAX_NATIVE_FILES`), validated `:298-300` | 20,000 | Same |
| Entry-wide open streams | `capture.rs:58`, enforced `sink.rs:425-427` | 128 | Masking memory: at most 16 KiB undecided per stream (`redact/stream.rs:122-125`) |
| Native bytes per poll | `goalcup-daemon/src/runtime/capture/native_plan.rs:11` | 256 KiB raw | One poll at about an eighth of the 8 MiB queue |
| Poll interval | `owner.rs:16` (`NATIVE_EVERY`) | 100 ms | Latency |
| Native finish read | `owner.rs:13` | 64 MiB | Seal time |
| Directory entries per poll | `native_capture.rs:281` | 8,192 | Poll CPU; not a loss, discovery resumes next poll (`:66-75`) |

### 5.2 Two traps that make "just delete the cap" unsafe

1. **The shared 128 bound latches native off.** With the native cap gone but `MAX_OPEN_STREAMS` at 128, the 129th open
   stream makes `check_transition` answer `Refusal::Limit` (`sink.rs:425-427`); the sink marks native lost
   (`sink.rs:276-283`); the emitter latches `Admission::Lost` (`native_capture.rs:970-973`) for the life of the
   adapter (`native_capture.rs:86-91`). No native capture for the rest of a 168-hour part. Can already happen today
   if wire streams alone push the total to 128 while native opens a file. Not seen in the run.
2. **A full queue latches native off too** (P14). This is the native twin of the wire bug, and worse, because the wire
   at least reconnects ten times.

A file is its own buffer, so the native reader never needs to lose anything under load: stop the pass and read the
same offset next poll. That is the backpressure ask 2 wants, at zero cost.

### 5.3 What removing the caps costs

- **File handles: nothing.** A native stream holds no handle between polls (`native_capture.rs:76-81`). Each poll
  opens a parent chain and one file, then closes them (`process`, `:1415-1500`; `open_chain` `:1376-1388`). Retained
  handles: one root per source (Codex 1, Claude Code 6). The run measured 17 to 18 open files for the whole daemon
  (E/log.jsonl `procs`). The macOS default of 256 is not at risk from native capture; the 32-handle budget
  (`goalcup-platform/src/handle_budget.rs:13`) can go with the caps (plan K9).
- **Memory: small.** A few hundred bytes per tracked file (est.): 10,000 files is a few MB. Per open stream at most
  16 KiB held, so 10,000 open transcripts is 160 MiB worst case; a quiet JSONL file should hold near nothing
  (unverified; measure).
- **CPU of the 100 ms poll: the real cost.** Every poll, every tracked file re-opens the parent chain (`fstatat`,
  `openat`, `fstat`, a mount check `statx`, `close` per level; `goalcup-platform/src/sourcecap/unix.rs:178-212,
  537-560`), then `fstatat`s the file. For Codex (three directory levels) that is about 16 calls per file per poll,
  about 170 per file per second. Estimates, not benchmarked.

| Tracked files | Calls per second today (10 Hz) | CPU at about 1.5 µs a call (Linux, est.) | With CH3 (parents once per pass, 1 s idle poll) |
|---|---|---|---|
| 43 (the run) | about 7,000 | about 1% of a core | negligible |
| 1,000 | about 170,000 | about 25% of a core | about 1,000 calls/s, under 1% |
| 10,000 | about 1.7 million | more than two cores: the poll cannot finish in 100 ms | about 10,000 calls/s, about 1.5% |

The poll runs on the owner thread, the same thread that drains the queue and writes the chain (`owner.rs:161-176`). A
slow poll slows the writer, and a slow writer filled the queue in this run. So removing the file cap without CH3
trades a visible loss for a new cause of `queue_full`. macOS lookups are slower (unverified factor).

### 5.4 Smallest change (one PR, about 60 src changed, about 250 test lines rewritten)

| # | Change | Where | Size |
|---|---|---|---|
| 1 | Delete `max_open_streams`, `max_files`, the two checks, `MAX_NATIVE_FILES`, the report fields `waiting_files`, `untracked_files`, `file_limit_reached` | `native_capture.rs:187, 264-267, 279-280, 293-300, 1323-1327, 1555-1558` | about -40 src |
| 2 | Native opens never hit the shared bound: raise `MAX_OPEN_STREAMS` (CH1: 16,384, a memory guard) or exempt native streams in `check_transition` | `capture.rs:58` or `sink.rs:425-427` | 1 to 5 src |
| 3 | Native never latches on load: a `room()` read on the sink checked before each read; no native frame is ever refused | `native_capture.rs:940-975`, `sink.rs:246-283`, the `CaptureSink` trait in `capture.rs` | about 20 src, 2 tests |
| 4 | Cheap poll (CH3): each parent once per pass, `fstatat` from that handle; 1 s when idle, at once when there is more work | `native_capture.rs:820-847, 1376-1430`; `owner.rs:16` | about 40 src |
| 5 | Any native loss still possible writes its count, paths and start time, not a bare stream-less gap | `owner.rs:139-145, 192-206` | about 15 src |
| 6 | End sweep: at the finish, read a still-pending file as a snapshot instead of counting it uncaptured | `native_capture.rs:869` | about 15 src |

CH1, CH2 and CH3 cover 1, 2 and 4 (`docs/plans/2026-10-30-cut-first-plan.md:154-156`). Items 3, 5 and 6 are new. Item 3
matters most: without it, removing the cap moves the failure from "11 files missing, gap at the end" to "every later
file missing, gap at the first refusal". Also remove the 64 MiB finish cap (CH12) and keep the 180 s finish budget
(`goalcup-protocol/src/control.rs:708`). Projection: 43 files per 2 hours is about 3,600 per 168-hour part, under
16,384; heavier use (2 to 5 times) gets close, so keep the plan's watch rule. The run's open streams peaked at 54.

What could go wrong: 100,000 matching files make the poll walk them all; with CH3 that is about 100,000 `fstatat` per
pass, about 15% of a core at 1 Hz. The 8,192-entry discovery window stays. Append-only streams never close before the
finish, so open native streams only grow during a part; fine with no cap.

### 5.5 What each harness writes about each call

| Harness | Per-call record | Call id | Model | Usage | Time | Subagents and parent link | Readable today |
|---|---|---|---|---|---|---|---|
| Codex | One `event_msg` `token_count` with `info` per successful response (rollout JSONL) | **None**: `TokenCountEvent` has only `info` and `rate_limits` ([protocol.rs at rust-v0.165.0](https://raw.githubusercontent.com/openai/codex/rust-v0.165.0/codex-rs/protocol/src/protocol.rs)) | Per turn (`turn_context`) | `last_token_usage` per call, `total_token_usage` running | `timestamp`, ms, UTC | One rollout per thread; child `session_meta.source.subagent.thread_spawn` gives `parent_thread_id`, `depth`, `agent_path` (all 42 children in E/final-reconcile.txt) | Yes, captured live |
| Claude Code | Assistant lines; count distinct `requestId` with non-zero usage and model not `<synthetic>`. Since 2.1.300 it also records each request's system prompt, tools and parameters (`docs/plans/2026-10-30-audit/D-harnesses.md:279-281`); shape unverified | **`requestId`** and `message.id` (fixture `spikes/claude-code-cli/fixtures/main-session.jsonl`: 19 assistant lines, 12 distinct real ids) | `message.model` | `message.usage` | `timestamp`, not monotonic (`docs/harnesses/claude-code-cli.md:54`) | `subagents/agent-<id>.jsonl` plus `.meta.json` with `agentType`, `description`, `toolUseId`, `spawnDepth` | Yes, captured live |
| Hermes 0.22.5 | `state.db` `messages` rows per session | Autoincrement only | Per session (`docs/harnesses/hermes-cli.md:62`) | Per session | Unverified | `sessions.parent_session_id`, `async_delegations` (`D-harnesses.md:960-1000`) | Only a final SQLite copy after exit |
| Amp | `T-*.log`, "not a transcript" (`connectors/amp-cli/harness.toml:2, 84-90`; `docs/harnesses/amp-cli.md:37`) | Unverified | Snapshots | Snapshots | Yes | Unverified | Captured live |
| OpenCode, OpenClaw | SQLite only (`origin/opencode-openclaw:docs/harnesses/{opencode-cli,openclaw-cli}.md`) | Unverified | Unverified | Unverified | Unverified | Unverified | No (schema 1 refuses SQLite) |

Codex and Claude Code give an independent call count and a full subagent list from session files alone. Claude Code
can join one-to-one to the wire through the `request-id` response header (captured but not yet seen in a kept header
stream, `docs/plans/2026-10-29-record-everything-phase2-notes.md:628-632`). Codex joins by count per time window.

### 5.6 How the observer found the loss

`measure.py` (225 lines) tailed the chain: WebSocket `response.create` with `generate` not `false`, keyed by
`thread_id` in `client_metadata`; HTTP `wire_request` opens labelled `POST /v1/responses`; 10-minute buckets.
`final.py` (about 30 lines) walked rollouts changed since the start: `session_meta`, `turn_context`, `token_count` with
`info`. Result (E/final-reconcile.txt): buckets match within 3 calls until 15:40, except 15:10 (181 harness, 236 wire:
rate-limit retries). Then 159 against 65, and 140, 135, 104 against 0. Lessons:

1. The counts measure different things. Codex counts successful responses; the wire counts attempts, including 95
   `response.failed` retries and 27 warm-ups (`generate: false`). A check looks for **harness above wire** in a window.
   Wire above harness is normal.
2. Buckets are clean; per-thread totals are noisy because HTTP POSTs were not attributed to threads (`wallet_api` 23
   harness against 45 wire, `consumer_api_ui` 52 against 3).
3. `secrets.py` (23 lines) decodes each 64 KiB segment alone, so a secret split across segments is missed. A built-in
   scan must join a stream's segments first.

### 5.7 A built-in reconcile

Run it in the CLI at the end of each part and on `goalcup status`, reading the harness home on local disk. It never
touches the chain, so it is independent of the code that failed.

| Piece | Rust src (est.) |
|---|---|
| Walk the connector's native roots, files changed since start, substring filter before `serde_json` | 30 |
| Codex: `token_count` with `info`; children from `session_meta`; first and last time per thread | 25 |
| Claude Code: distinct `requestId`, model not `<synthetic>`; subagent files and meta | 25 |
| Compare per 10-minute bucket with the written count; print windows where harness is above recorded | 40 |
| Hermes, Amp, others: "not checked for this harness" | 0 |
| **Total** | **about 130 src, about 120 tests** |

Example finish line (point 6):

```
Codex's own log: 1,261 model calls by 43 agents (1 main, 42 subagents).
Recorded: 842 calls. Not recorded: about 419, from 15:43 to 16:54 (1 h 11 min).
Session files read: 32 of 43.
```

Limits: it needs the honest written count from ask 4. Harness formats are internal (Claude Code says the format
"changes between versions", `D-harnesses.md:276-278`): if the reader finds zero calls but the wire has calls, print
"could not read the harness's own log", never "everything recorded". It is a display, not evidence: local files can be
edited, so the server repeats the count from uploaded native files (point 5). It must not slow the finish: rollouts
grew about 30 MB an hour (58.5 MB in 2 hours), about 5 GB a week (linear, unverified); read only lines with
`token_count` or `"assistant"`, keep a 20 s budget, say "partly checked" when it runs out. Run it every few minutes
in `goalcup status` too: it would have shown this loss at 15:53, not 16:54. Plan AD4 (`cut-first-plan.md:182`, about
220 src) is the organizer kit tool; this is its participant half and can share the readers. Start both from the
observer's scripts. Wording must say what was missed and what was kept, never more.

### 5.8 Copy session files at the end instead of live

`final_snapshot` already exists (`native_capture.rs:49-54`; Hermes uses it, `connectors/hermes-cli/harness.toml:115-121`).

| | Live (caps removed) | Copy at end only | Live plus end sweep (recommended) |
|---|---|---|---|
| Code | Keep `native_capture.rs` (1,632) and `sourcecap` (1,285); net about -40 | Little saved: discovery, no-follow reads (point 4: a planted symlink), sink and masking stay; maybe 300 lines go and much of `goalcup-daemon/tests/it/capture_native.rs` (2,664) is rewritten | Live plus about 15 lines |
| Crash before finish | Everything up to the crash is chained | Nothing native; in a non-persistent sandbox the files are gone | As live |
| Harness deletes or rewrites files | Read bytes kept; change is a `SourceChanged` gap | Lost without trace (Claude Code `cleanupPeriodDays` default 30 days and a 2.1.310 fix for deleting recent files, `D-harnesses.md:331`; `.superseded`/`.orphaned`, `claude-code-cli.md:54`; Hermes prunes after 7 days; Codex rollout compression, off in 0.165, `D-harnesses.md:144`; the agent itself) | As live |
| Agent edits its transcript | Shows as `SourceChanged` | Undetectable | As live |
| Seal time | Small backlog | About 5 GB a week of Codex through masking in a 180 s budget (est.): likely too slow | Small |
| Upload | Spread | One burst at the end | Spread |

Periodic whole-file copies grow with the square of the run length; copying only new bytes is what the live reader
does. Keep `final_snapshot` for SQLite stores only.

### 5.9 The owner's idea: register the endpoints, proxy them, check local logs

The run supports it. The native reader kept recording subagent files for 27 minutes after the wire died. The 43
rollouts were 58.5 MB against 637 MB of wire payload for the 842 recorded of 1,261 calls: the local logs are the cheap,
deduplicated story; the wire is the size problem. What failed was the plumbing between the proxy and the recorder (two
8 MiB queues, a socket close as the only signal, a failure budget built for a dead daemon). The idea's full form,
the proxy writing the record itself (option E above, plan K13), removes that hop and is about 12,000 source lines of
change: after Edition 01. For Friday: fix the hop (asks 1 to 4) and add the reconcile (5.7), which is the "check against
local logs" half of the idea.

## 6. Plan gaps (from the N3 gap check)

### 6.1 What the run adds to the facts the plan used

| # | Fact | Plan knew? |
|---|---|---|
| F1 | Daemon queue 8 MiB, shared, a frame that does not fit loses the whole producer (`sink.rs:110-113, 268-283`) | Partly: CH1 raises it for memory, not load |
| F2 | The proxy queue has the same bound (`proxy/ingest.rs:29-33, 122`). A proxy-side `QueueFull` is not counted as a failure (`:57`); a daemon-side one arrives as a remote close and is (`:1104-1135`) | No |
| F3 | Ten losses, epochs under 60 s, `latch_terminal` (`proxy/ingest.rs:104, 108, 459-470`) | CH6 deletes the budget but does not link it to loss |
| F4 | The burst came from provider rate limits; any participant on a low tier hits it | No |
| F5 | The native reader shares the queue and latches for life (`native_capture.rs:84-90, 970-973`) | No |
| F6 | 32 slots, 43 files; 11 subagents from 16:44 have no wire call and no native stream | Yes, CH2; now more urgent |
| F7 | Finish counts at selection, every WS client message, 37 interrupts (`proxy/http.rs:305-320`, `proxy/websocket.rs:490-505`, `proxy/api.rs:397-403`) | No. AD2 reuses the counter |
| F8 | No seal gap for `sink_refusals` (`writer.rs:184-199`) | No |
| F9 | Seal `limit_exceeded` gap has no stream (C8) | No |
| F10 | One stream per WS message; chain 1.8x payload | No |
| F11 | 1.30 GB chain, 405 MB compressed for about 61 minutes of recording; 3.2 to 1, not "about 10x" (`chunk.rs:17-19`) | No; section 9 says only "under 64 GiB" |
| F12 | Upload cadence and backoff (`policy.rs:80-90`, `chunk.rs:13`); 94 MB at 15:55 | No; K10 cuts only leftovers |
| F13 | Seal prune (`prune.rs:22, 120`) broke `verify:` and the full secret scan | Partly: CH10, Q9. The plan's `cp -a` every 10 minutes was not done |
| F14 | No push. `goalcup status` says "Your prompt starts with these lines" (`goalcup-cli/src/commands/status.rs:478`) when it did not. Codex: "no Git remote is configured" | No; K19 is after Edition 01 |
| F15 | Codex shows "1 warning" at start: the `-c` overrides force embedded mode (`connectors/codex-cli/harness.toml:43-47`; E/start-pane.txt) | CH15 keeps the args; AD8 does not explain the warning |
| F16 | Load: daemon 50 to 52 MB RSS, 17 to 18 open files, 9 to 17% of a core lifetime average; `goalcup start` 19 to 67 open files; Codex 320 to 385 MB, up to 97 open files; disk free above 61 GB | Plan had no long-run data; peak CPU still unknown |
| F17 | Support block on the finish screen | AD2 replaces only "Recording works" |
| F18 | `auth_refused=1`: one connection failed proxy authentication, cause unknown (`host.log`) | No |

### 6.2 Plan items judged against the run

| Id | Verdict | Why |
|---|---|---|
| K1 decoders | Still right, missing something | The observer did not use them; its scripts (`measure.py` 225, `final.py` 28, `secrets.py` 23 lines) are a working AD4 prototype. Join segments before scanning |
| K2, K4, K6, K8, K9, K12, K26 | Still right | No relation, or owner said yes (K8: Q6). `handle_budget.rs` goes with CH2. K12: 218 GB a week is under the 1 TiB writer quota (`capture.rs:54`) |
| K5 disk | Still right, missing something | Disk use now depends on upload lag: 30 GB free and a stalled upload fill in about a day. Optional: one disk-free number in `goalcup status` (a read, not handling) |
| K7 probe | Still right | It checks the provider, not the recorder; it would not have caught this |
| K10 uploader leftovers | Still right, missing something | Add the cadence and backoff change (G7) |
| K11 outage windows | **Narrow it** | Its range `proxy/mod.rs:647-920` includes `ProxyHealth`, `Counters`, `sink_answer` and `sink_refusals` (`proxy/mod.rs:604-624, 904-914`), the only proxy-side proof of drops after the latch. Cut `proxy/outage.rs` and outage fields only |
| K13 daemon merge | Defer, but the run is new evidence for it | Every loss was on the proxy-daemon hop |
| CH1 bounds | More urgent, missing something | The queue raise is now a loss fix: move it to the first PR. Stream and connection raises stay in PR8a |
| CH2 native caps | More urgent | 11 transcripts lost |
| CH3 cheap poll | Still right | One suspect for the slow drain; measure (G9) |
| CH4 one-stream refusal | Missing something | Apply the same rule to `QueueFull` (ask 2) |
| CH5 open-file limit | Still right, less urgent | 17 to 18 daemon files; Mac with 256 still untested |
| CH6 proxy never refuses | **More urgent, split it** | Move the `MAX_CONNECTION_FAILURES` part (and `STABLE_EPOCH`, `proxy/ingest.rs:108, 1116-1124`) to the first PR. Missing: `MAX_PRODUCERS` (G2) |
| CH10 prune on ack | Missing something | Fix `verify:`; test runs keep a full copy |
| CH14 Worker config | **Now wrong in one part** | Remove the run limit (owner Q2/Q3): set it very high in `wrangler.toml:59, 151`. Keep `model_requests` in the heartbeat as the "harness active" signal |
| CH15 connector data | Missing something | Add the `-c` warning to AD8. Codex already sets `check_for_update_on_startup=false` (`connectors/codex-cli/harness.toml:44`), which answers the owner's Q7 for Codex |
| AD2 Recorded line | **Now wrong as written** | Reads `model_calls` (`run/mod.rs:266, 716-719`). Rewrite as G4 |
| AD3 "Recording has stopped" | More urgent | Would have caught 15:43. After G1 the trigger becomes "no live epoch for more than 60 s" plus the old daemon test |
| AD4 `live-check` | More urgent | Compare "wire minus failed responses" with the harness count per bucket (15:10 bucket: 236 against 181) |
| AD6 tests | Missing something | Neither N1 nor N2 reproduces the run: add N6 (G9) |
| AD7 N4 server rebuild | More urgent | The only automated proof of point 5; the run could not check the server copy |
| AD8 participant text | Missing something | Codex shows one expected warning; a heavy run makes about 1.3 GB an hour; push lines at the top of the prompt; recording may pause under load and the screen says when |
| PR8a | More urgent | Before PR4 and PR5 |
| PR7 | Missing something | Loses the budget part to the first PR; K11 narrowed |
| PR9 | Wrong in order | AD2 and AD3 move to Saturday as G4 |
| PR6, PR10, PR11 | Missing something | `verify:` fix; G7; AD8 warning line |
| Q2 | Answered | 16,384 means files or messages open at the same time, not subagents in total. A finished subagent's file holds its slot until the part ends. Run peak: 54 |
| Q3 | Now wrong | Remove the 64 GiB limit; the run's rate reaches it in about 168 hours |

### 6.3 What the run found that the plan does not cover

| Id | Gap | Smallest fix | Point | Size (est.) | What could go wrong |
|---|---|---|---|---|---|
| G1 | A load drop counts toward "the daemon is dead" | Ask 1; queue to 64 MiB both sides with reserves scaled (CH1 numbers) | 1, 2, 3 | about 40 src, about 150 tests rewritten | A dead daemon is retried for ever; log once a minute |
| G2 | `MAX_PRODUCERS` (65,536, `capture.rs:63`) is a second terminal switch; a week at one drop per 10 s spends about 60,000 | Forget superseded producers, or 1,048,576 with its memory cost | 1, 3 | about 20 src | Forgetting could weaken the "one wire lineage" check; test with the verifier |
| G3 | No seal gap for proxy refusals | Ask 3 item 8 | 3, 5, 6 | about 5 or 40 src | Two gaps for one loss |
| G4 | Count at selection | Ask 4 | 6 | about 60 src, 60 tests, one `CONTROL_API_VERSION` bump | Two counts can confuse; show only the written one |
| G5 | One full queue kills every in-flight stream of a producer: 14 streams ended `interrupted`; all 20 subagents shared one proxy producer | Ask 2 (about 40 src per M1; N3 estimated about 200 src, 300 tests) | 1, 3 | see ask 2 | Mid-body gaps in decoders |
| G6 | Native latched for good on a full queue | Ask 3 item 6 | 2, 3 | about 20 src, 1 test | A long-full queue delays transcripts; the "not read yet" count shows it |
| G7 | Upload lag | Section 4.7 plus long-window zstd; measure the gain on a saved trace before shipping | 5 | about 10 src | More D1 rows and R2 objects; about 128 to 140 MiB uploader memory; errors may be Vercel-specific (unverified) |
| G8 | Trace size | Accept, remove the limit, plan storage (4.8); per-direction WS and dedupe later | 5 | config now, about 300 src later | Format change touches verifier and decoders branch |
| G9 | No test reproduces the load; peak CPU unknown | Test N6: 20 to 40 producers, 1.5 MB bodies in bursts, 10 minutes, real owner; assert no terminal latch, every loss gap has a recovery, native stays live; print drain MB/s and peak CPU. Also benchmark `StreamRedactor` plus `ChainWriter` on 1.5 MB JSON bodies on 4 vCPU: if below about 30 MB in one second, spill (option D) moves up | 1, 2, 3 | about 150 tests | Machine-dependent; serial group |
| G10 | `verify:` line fails after prune | 4.9 | 6 | about 5 src | None |
| G11 | Push lines missing from the prompt | Status wording "Start your prompt with these lines"; AD8. Option: `goalcup start` runs `git init` and adds the remote and helper | 6, 7 | about 5 src, or about 30 | The option writes into the workspace and may clash with an existing repo |
| G12 | Full trace never scanned | Next run keeps a full copy and scans it; AD4 scans the server copy; joined segments, raw, base64, percent-encoded; print counts, never values | 4 | none | Needs the real key on the machine |
| G13 | Codex `-c` warning | One AD8 line | 6, 7 | text | A later Codex could turn it into a prompt that blocks chat mode |
| G14 | Seal `limit_exceeded` names no limit | After CH2 check which cause remains | 2 | none now | If it is the capture table, CH1's connection raise fixes it |
| G15 | `auth_refused=1` | Look once: which process reached the proxy without its credential | 1 | none | Could be the "silent bypass" risk |

### 6.4 Planned cuts that would remove something the run needed

Only K11, and only its counters: narrow it as in 6.2. K7, K5, K1, K12 stay cut. CH10 stays for participants. K19 keeps
the push lines through Edition 01; fix the wording now.

### 6.5 Revised order of work

Rule: fixes that stop silent loss land first. Pure deletes run in parallel because they do not touch the recording
path. If time runs out, cuts slip; loss fixes do not.

| Day | Work |
|---|---|
| Thu 29 Oct | PR1 docs (owner Q1) |
| Fri 30 Oct | **PR-S1 "never off for good":** ask 1 (G1), queue raise to 64 MiB both sides, base64 ingest bytes, G2, G3 first form, G6 (native waits), P11 and P13 fixes, test N6 (G9). In parallel: PR2 |
| Sat 31 Oct | **PR-S2 "say the truth":** ask 4 (G4), AD3 rewritten, support block off the screen, G10, G11 wording, the reconcile (5.7). **Ask 2 gap substitution** (both queues), CH4, P10 dedupe removed. **PR8a:** CH1 streams and connections, CH2, N1. **PR-S3:** upload backoff, cadence, long-window zstd; CH14 with the limit removed (owner deploys) |
| Sun 1 Nov | **Second 2-hour run** on main (section 7). In parallel: PR3, PR4 |
| Mon 2 Nov | Fix what run 2 found. PR5, PR6 (with `verify:` fix), PR7 (CH5, rest of CH6, K11 narrowed) |
| Tue 3 Nov | PR8b (CH3, CH12, K12), PR9, PR10, PR11 (AD5, CH15, AD8 lines). Feature stop 4 Nov 00:00 UTC |
| Wed 4 Nov | PR12 (N4). Tag the candidate. Third 2-hour run |
| Thu 5 Nov | Buffer. PR13 only if green |
| Fri 6 Nov | Release. Worker config before 7 Nov 00:00 UTC |

If run 2 still shows interrupted streams from `queue_full`, the cuts PR3 and PR4 slip after Edition 01 to make room.
If the owner thread is the bottleneck (low drain MB/s), CH3 and per-direction WS server streams move up; measure
first. After Edition 01: per-direction WS (if not done), dedupe, K13 daemon merge.

## 7. What the next 2-hour run must prove

### 7.1 How to run it

- Same shape: Codex, API-key login, a prompt that asks for many parallel subagents, at least 2 hours of real load.
  No 45-minute idle (this run sat from 15:58 to 16:43 waiting for "keep going"): script the "keep going".
- Claude Code in a second sandbox if one is free.
- Keep a full local copy: every 5 minutes `cp -al` the trace folder on the same filesystem. Hard links keep each part
  after prune deletes its name, at no extra disk cost. Replaces the plan's `cp -a` every 10 minutes (1.3 GB a copy).
- Arrange the server copy first: the session holds `DATABASE_URL` and R2 read credentials for `fetch-run.sh`, or the
  owner runs it right after the seal.
- Sample CPU every 10 s from `/proc/<pid>/stat` deltas (peak, not lifetime) for the daemon, `goalcup start` and the
  harness. Log owner drain MB/s if the build exposes it.
- Push lines at the top of the prompt.

### 7.2 Pass criteria

Numbered T1 to T16 so they do not clash with the P numbers in section 2.

| # | Point | Check | Pass |
|---|---|---|---|
| T1 | 1, 3 | Wire calls against the harness's own record per 10-minute bucket, whole run | Every bucket has wire calls. Wire minus failed responses equals the harness count in each bucket, or each difference is named. No bucket after any gap stays at 0 |
| T2 | 3 | Never off for good | Every `queue_full` or `ingest_lost` gap is followed by a `recovery` within 60 s. No terminal latch in `host.log`. Ideally zero producer-level `queue_full` drops under the same burst (only per-stream gaps) |
| T3 | 3, 5 | Loss never silent | Every period with relayed model traffic and no recorded frames is covered by a gap with a start and an end, checked by matching harness call times against gap spans |
| T4 | 6 | Honest count | The finish and `goalcup status` count equals the model requests in the chain (from `live-check`), exactly. With any gap, the screen says how long recording was off, with times, never "Recording works" |
| T5 | 6 | Built-in reconcile | The finish line prints the harness's own count, the recorded count and any not-recorded window; it agrees with the observer's buckets |
| T6 | 2 | Every subagent | Every rollout (or Claude Code subagent file) on disk is in the chain by path. Every subagent thread has wire calls. No stream-less `limit_exceeded` gap |
| T7 | 2 | Native survives load | No native loss gap. Native bytes in the chain equal the session files' bytes at the seal, within what the last poll had not read |
| T8 | 5 | Upload keeps up | At every 10-minute tick the server is at most 10 minutes or 64 MiB behind. No backoff wait over 2 minutes. Seal acknowledged within 1 minute |
| T9 | 5 | Server rebuild | `fetch-run.sh` output equals the hard-linked local copy byte for byte. `goalcup-verify --capture-integrity` on the server copy exits 0 or names only gaps already in T2 and T3 |
| T10 | 4 | Secrets | No API key, GitHub token or `Authorization: Bearer` in the full local copy and the server copy, raw, base64 and percent-encoded, segments joined. A fake `.env` the agent prints and a URL with a password are masked |
| T11 | 5 | Size | Chain and compressed bytes per hour by source (HTTP bodies, WS client, WS server, native), the compression ratio with the long window against 3.2 to 1, and the 168-hour projection. No pass line (the limit is gone); it feeds G8 |
| T12 | 3 | Load | Peak daemon CPU per 10 s, drain MB/s, RSS (with the 64 MiB queues and long-window uploader), open files. Per-call latency through the proxy. No refused model connection |
| T13 | 6 | Participant screen | No support block, no `verify:` that cannot work, no internal words (`anchor`, `terminal record`, `seal.json`, `admission`). The Codex warning is the one AD8 names |
| T14 | 7 | Push | The GitHub repository has the agent's commits by the end |
| T15 | 3 | Recorder faults | Kill the daemon once: "Recording has stopped" within 2 minutes, recording resumes after `goalcup continue`. One manual 2-minute network cut: recording goes on, upload catches up |
| T16 | 1 | Unknowns from run 1 | Name the seal `limit_exceeded` cause if one remains (G14), explain `auth_refused` (G15) and pair every `queue_full` gap with its recovery (C10) |

A run that fails T1, T2, T3 or T6 blocks the release candidate. The others are fixed on the buffer day or named in AD8.

## 8. Blind spots and what is not verified

- Superseded by section 9: the server copy has since been fetched and read. At the time of writing, which producer each of the 22 `queue_full` gaps
  belonged to, why 5 have no recovery, whether native ever latched, and that the 11 missing transcripts are the last
  11 subagents are inferred from timing, slot order and code.
- Owner thread throughput, peak queue depth and peak daemon CPU were not measured. If the owner was saturated, a
  bigger queue only delays the drop; per-stream gaps, a cheaper poll and fewer records per second are then needed.
- The 3.6x encoding figure comes from a text sample. The compression ratios come from synthetic text; the real
  long-window gain is unknown until measured on a saved trace. That the `zstd` 0.13 crate exposes the two setters with
  default features off is unverified.
- Native poll CPU and memory figures are system-call counts times typical costs; benchmark 1,000 and 10,000 fake
  rollouts on Linux and macOS before Friday.
- Gap substitution changes what the verifier sees (many mid-stream `QueueFull` gaps instead of a few producer
  losses). The provider decoders were not re-read for mid-body gaps.
- Removing the failure budget means a broken daemon is retried for ever. Intended, but log once a minute.
- One harness, one sandbox, one model setting. Claude Code's load shape (no WebSocket, different subagent files) is
  untested at this scale. The upload network errors may be specific to the sandbox.
- Codex `token_count` per successful response is measured on this one run, not a documented contract; a Codex update
  could change it. The Claude Code 2.1.300 per-request record was not seen in a fixture.
- Storage prices are from the R2 pricing page on 29 Oct 2026 and can change. A 168-hour run at the busiest rate is a
  projection, not a measurement.
- A reconcile line saying "Not recorded: 419" could alarm a participant whose run is still valid; the wording must say
  what was kept as well as what was missed.

## 9. Server copy

Source: the "Server copy" section of the run report at commit `74daaa03` on `origin/smart/tender-clarke-qupne3`
(`docs/research/results/2026-10-30-two-hour-run.md`, from line 123). A reviewer with read-only R2 and database access
rebuilt the run with `sh scripts/fetch-run.sh r632072599ee7b6fc <outdir>` (R2 GETs only, the index query in a
`READ ONLY` transaction) and compared it with Codex's 43 rollout files. The verifier was built at `a31094c`, the run's
own build. Every check printed counts only. The code facts below were checked at the working tree:
`git diff a31094c HEAD -- crates server` is empty, so the lines cited are the code that ran. This section replaces the
inferences in sections 1 to 8 wherever the two differ; the table in 9.8 lists each correction.

### 9.1 The whole chain is on the server

| Fact | Value |
|---|---|
| Chunks fetched | 20, each matching its SHA-256 and stated length |
| Chain bytes | 1,304,968,017, seq 0 to 479,319: the same count the local finish reported |
| Offsets | Run on with no hole; the last chunk meets the committed final heartbeat |
| Ack lag | The chunk acked at 15:34:47Z ends at seq 341,077; the next ack came at 16:26:11Z |
| Verifier `--capture-integrity` | Exit 1, `incomplete`, anchor `signed`, sealed, 479,320 records, 23 gaps; findings `gap_recorded`, `producer_recovered`, `producer_unclosed`; ack verified through seq 479,319 (16:54:46Z), no torn tail, 0 records after the deadline; termination `confirmed` |

So point 5 held for run 1: prune at the seal lost nothing that the server did not have, and `fetch-run.sh` rebuilt
the run by ID and time. What the server copy cannot hold is what was never recorded (9.3, 9.4). The 51-minute ack
lag matches three backoff waits on one chunk, 987, 975 and 1,083 s, 50 min 45 s in total (4.7). With the 0 to 25%
jitter (`goalcup-cli/src/upload/policy.rs:283-291`), a 2-minute cap can still wait 150 s, so "no wait over 2 minutes"
is not a valid pass line; use 2.5 minutes.

### 9.2 Secrets and a side effect of masking

- **Zero hits** for the run's OpenAI key and GitHub token in the whole 1.3 GB, with each stream's segments joined in
  `source_offset` order: 374.0 MB of HTTP request bodies, 158.0 MB of WebSocket client messages, 103.5 MB of server
  messages, 48.0 MB of native streams, 36.2 MB of response bodies, 0.9 MB of headers, the open-record metadata, the
  five side files and the 43 raw rollouts. No body was compressed on the wire.
- **Positive control:** all 378 `authorization` request headers are `[masked]`. No unmasked `Authorization`,
  `Proxy-Authorization` or `x-api-key` value anywhere.
- **Side effect:** the daemon's audit shows 634 `openai_key`, 777 `huggingface_token` and 1 `github_token` spans. In
  the raw rollouts the same two rules match 103 times, and 102 of those sit in the middle of Codex's
  `encrypted_content` reasoning blobs. The one `github_token` span is also inside such a blob. None is a credential.
  No shared rule has a left boundary (`crates/goalcup-daemon/src/redact.rs:195-311`), and 13 vendor-prefix rules can
  fire inside a base64url run. The recorded copy of those blobs is altered. The wire and the harness's own files are
  never changed (`crates/goalcup-cli/src/proxy/http.rs:3-9`, `proxy/websocket.rs:3-8`;
  `crates/goalcup-platform/src/sourcecap/unix.rs:222-227` opens read-only), so Codex kept working.
- **The real risk is F-07, not the altered blob.** `openai_key` and `google_key` have a class that covers the whole
  base64url alphabet, so a match runs to the end of the blob. If that is more than 16 KiB, the stream redactor holds
  past `MAX_HELD_BYTES` (`crates/goalcup-daemon/src/redact/stream.rs:120-125`), fails closed and masks to the next
  `\n` (`docs/CHEAT-CHECKLIST.md:178-181`). A WebSocket client message is one JSON line, so one such hit masks the
  rest of a model request. That loses recorded context (point 1), not a secret. Whether any run 1 span did this is
  unverified: the report gives span counts, not lengths.
- **A plain `\b` is the wrong fix.** It misses keys after JSON escapes (`\n`, `\t`), percent escapes and ANSI codes,
  and the stream pass sees no byte before its window (`redact/stream.rs:2082`). The correct form is a guard on the
  byte before a match, with escape exceptions: about 80 to 110 src and 120 to 180 test lines across `redact.rs`,
  `redact/stream.rs` and `spikes/redact.py`. It stops catching a key glued after a letter, digit, `_` or `-`.

### 9.3 When recording stopped

- **No proxy record after 15:43:15.44Z.** The last is the `queue_full` gap at seq 478,379 (producer `fd1a2b77`). The
  940 records after it are all from the daemon: native to 16:54:43Z, the `limit_exceeded` gap, `producer_closed` and
  `local_finish`.
- **18 proxy producers, 22 `queue_full` gaps.** One gap each, except three producers with 2, 2 and 3. Each later
  producer starts at a `recovery`; 17 recoveries for 17 later producers. The last gap has none. None of the 18 has a
  `producer_closed` record, which is why the verifier says `producer_unclosed`. Every gap has `lost_bytes: null`.
- **The budget, confirmed.** After `ee7b9797` lived 161 s, nine producers in a row lived 3 to 45 s. The drop at
  15:43:15 was the 10th failure in a row.
- **Native slots, confirmed.** The `limit_exceeded` gap (seq 479,285, 16:54:43.810Z) has no stream. It is the native
  finish marker (`crates/goalcup-daemon/src/runtime/capture/owner.rs:143-145`). All 32 native streams stayed open to
  the finish. The 32 captured files are the first 32 by start time; the 11 missing are the subagents started from
  16:44:01Z to 16:53:20Z. They hold 136 `token_count` events. For those 11 threads the trace has neither leg.

### 9.4 Model calls

| | Count |
|---|---|
| Wire model calls | **842** (599 generating WebSocket `response.create`, 243 HTTP `POST /v1/responses`, 11 of them cut off), 14:54:41Z to 15:43:13Z. Not counted: 27 warm-ups (`generate: false`), 37 `response.interrupt` |
| Codex `token_count` with usage | **1,261** in 43 files |
| Before 15:43:15.44Z | Codex 788, wire 842. The extra 54 are mostly rate-limit retries in the 15:10 bucket: about 97 `rate_limit_exceeded` `error` events and 99 `response.failed`. From 15:40:00Z to the cutoff both sides have exactly 65 |
| After 15:43:15.44Z | Codex **473** (37.5%), wire **0** |
| Threads | 28 of 43 rollout threads have wire calls; 21 match exactly, 4 have more on the wire (retries), 3 have less. 1 wire-only thread (1 call). The 15 with no wire call: 4 subagents started 15:48:30Z to 15:56:43Z and the 11 from 16:44Z |

So before the cutoff the wire matched Codex in every bucket once retries are allowed for. Per-thread matching works:
every HTTP POST carries a `thread-id` header, and WebSocket messages carry the thread id in `client_metadata`. This
corrects section 5.6 lesson 2, which said HTTP POSTs were not attributed to threads.

### 9.5 Verifier blind spots

| Field | Said | Why | Code |
|---|---|---|---|
| `analysis.cross_leg.state` | `consistent` | It judges only zero against non-zero. The wire's last request time is computed but ignored; there is no native time range. A wire that stops 71 minutes before native ends reads `consistent` | `crates/goalcup-verifier/src/capture_analysis/report.rs:326-333, 375-386`; `session.rs:193-241` |
| `provider_errors.total` | 0 | It counts only the token-source leg. The wire decoder was degraded (429 unknown records), so native won, and the Codex native decoder emits no error at all | `stats.rs:749-779, 907, 949-956, 1023`; `native/codex.rs` has no `kinds::ERROR` |

Only `producer_unclosed` hinted at the loss. Both fields sit in files that K1 deletes, but K1 now waits until after
Edition 01, so judges will read them. `docs/CHEAT-CHECKLIST.md:46-47` leans on `cross_leg` as a bypass check; it fires
only when the wire has zero streams. Run 1's observer scripts do not replace them either: `final.py` hard-codes paths
and a start time and has no pass rule; `secrets.py` decodes each segment alone.

Smallest patches: a `wire_ended_before_native` state when native's last observation is more than 10 minutes after
the wire's last request (about 20 src, the unit test, the integration test and six byte goldens in
`crates/goalcup-verifier/tests/fixtures/m2-report-bytes/`); and the wire leg's error total printed beside the primary
one (about 10 src). Caution: the OpenAI decoder emits an error both for an `error` event and for `response.failed`
(`openai.rs:475-486, 545-558`), so a naive count may double to about 196. Whether they pair one to one is unverified.

### 9.6 What the server showed

- **Run** `r632072599ee7b6fc`: `flagged_at` 15:29:25Z, `flag_reason` `gaps`, status `open` (by design: a segment
  sealed before `ends_at` leaves the run open).
- **Entry:** sealed 16:54:49Z, `model_requests` **1,766** stored. That is the proxy's relay-time counter
  (`goalcup-cli/src/upload/uploader/heartbeat.rs:61`, `goalcup-cli/src/proxy/api.rs:342-358`), wider than the
  finish's 1,736 (`generation_requests`): it adds about 30 other requests on a captured host (inferred split).
- **Heartbeats:** 22. `recording` is `unknown`, then `ok` to 15:14:23Z, then `gaps` from 15:29:25Z. Never `failed`.
  The word is the daemon's view only (`goalcup-cli/src/run/watch.rs:270-273`, `goalcup-cli/src/run/mod.rs:114-124`);
  the proxy's latch lives in the CLI and nothing passes it on. Even a `failed` would not re-flag: `note_heartbeat`
  sets the flag only while `flagged_at IS NULL` (`server/worker/sql/goal-cup-tracer-upload.sql:2831-2832`).
- **The unread tell:** from 15:44:27Z to the final heartbeat `model_requests` rose by 600 (1,166 to 1,766) while the
  chain grew 16.9 MB, all native: about 28 KB per new request against about 1.1 MB per request before. From 15:59:30Z
  to 16:39:33Z nine heartbeats had the same `seq` (Codex was idle then).
- **Notices:** `started` and one `recording` (15:30:49Z, "gaps"). The `recording` notice has dedupe key `""`
  (`server/worker/src/notices.ts:150`), so a run gets one, ever. No notice kind exists for "recording stopped"
  (`server/worker/src/db.ts:396-397`). No dashboard reads the flag, heartbeats or notices.

The smallest signal for Edition 01 is client-side: send `failed` in the heartbeat when the wire has been off 60 s or
the recorder is lost (about 10 src in `watch.rs`). A run whose first bad heartbeat is `failed` then gets today's
email worded "(failed)". A run that said `gaps` first, as run 1 did, gets no second email unless the website SQL lets
`failed` replace `gaps` and the Worker's dedupe key includes the reason (about 8 SQL lines, 2 Worker lines). An
organizer query on `goal_cup_tracer.heartbeats` (requests rising while bytes per request fall tenfold below the
entry's own average) needs no deploy; on run 1 it would have fired at about the 15:54 to 15:59 heartbeat (inferred,
not run).

### 9.7 Size and compression

| Part | MB | Share |
|---|---|---|
| Payload | 721.8 | 55% |
| Base64 growth | 239.3 | 18% |
| Record framing | 343.9 | 26% |
| **Chain** | **1,305.0** | |

| Stream source | Streams | Chain MB | Payload MB |
|---|---|---|---|
| HTTP request | 758 | 506.3 | 375.4 |
| WebSocket server message | 152,728 | 465.7 | 103.5 |
| WebSocket client message | 671 | 214.3 | 158.2 |
| Native rollout | 32 | 66.6 | 48.2 |
| HTTP response | 702 | 51.0 | 36.4 |

- **Per-message WebSocket cost, measured:** both WebSocket sources take 680.0 MB of chain for 261.7 MB of payload, an
  overhead of **418.3 MB**. Server messages alone: **362 MB** (215.1 MB of open and end records, 112.3 MB of segment
  framing, 34.8 MB of base64). The external review's "about 466 MB" was the whole `ws_server` chain share, payload
  included.
- **Long window, measured on the real chunks:** recompressing each stored chunk's byte range at `zstd -3` gives
  408.9 MB (the 405 MB sent, 3.2 to 1). With `zstd -3 --long=27` it is **100.8 MB, 12.9 to 1, about 4 times
  smaller**. By chunk: the 264 MB mostly-WebSocket chunk goes from 8.7 to 11.4 to 1; an 88 MB mostly-HTTP chunk from
  2.7 to 23.1 to 1. The synthetic 14.5 to 37 to 1 (4.5) was an upper range: the real ratio is below its low end, and
  the real gain (4 times) is a little above the synthetic 3.6 times.
- **Not measured:** the gain at the shipped chunk size. CH17 makes a chunk about 5 minutes (about 100 MB of chain at
  run 1's rate); each chunk is its own frame, so the gain may be smaller. The 128 MiB window is needed to decompress;
  the Worker never decompresses (`server/worker/src/chunks.ts:165-225`) and `fetch-run.sh`'s `zstd -dcq`
  (`scripts/fetch-run.sh:327`) accepts a 2^27 window by default ([zstd manual](https://github.com/facebook/zstd/blob/dev/programs/zstd.1.md)).
  Memory on the client: about 140 MiB more while a chunk is built (synthetic, 4.5).

**168 hours, revised.** At run 1's busiest rate (about 218 GB raw a week), 12.9 to 1 gives about **17 GB** compressed
per busy week, not 19 to 34 GB: about $0.25 a month and $1.52 for 180 days per run, $152 for 100 entries and $760 for
500, under the 180-day lock. Smaller chunks may compress less, so read 17 GB as the low end.

### 9.8 Corrections to this appendix

| Where | Said | Corrected by the server copy |
|---|---|---|
| Summary, N, plan | About 470 of 1,261 calls lost | **473** Codex calls after 15:43:15.44Z had no wire record (37.5%). Wire total 842 against 1,261, a difference of 419, because the wire's 54 retries before the cutoff offset part of the loss |
| 1.1 step 1 | 95 calls failed `rate_limit_exceeded` | About 97 `error` events with `rate_limit_exceeded` and 99 `response.failed` in the chain |
| 1.1 step 12 | The heartbeat's `model_requests` is the same "seen" count | It is a wider counter: 1,766 stored against 1,736 printed (9.6) |
| 1.1, after the table | The 11 late subagents left nothing on the server (probable, unverified) | **Verified:** no native stream and no wire call for those 11 threads |
| C8 | The seal `limit_exceeded` gap is almost certainly native | **Verified:** native finish marker, 32 streams held to 16:54:43Z |
| C10 | 5 of 22 gaps have no recovery, 4 unexplained | **Explained:** 18 producers; three had 2, 2 and 3 gaps; 17 recoveries; only the final gap has none |
| Ask 4 risk | "The server keeps its own count from the chain" | **False.** The Worker never opens a chunk (`server/worker/src/chunks.ts:171-225`). Its only number is the heartbeat's seen count |
| 4.1 | Per-direction WS server streams save about 218 MB of open and end records and 300 to 320 MB in all | Open and end records of server messages: **215.1 MB**. Total per-message overhead: **362 MB** for server messages, **418 MB** for both directions. The external review's 466 MB included payload |
| 4.2 | 720.8 MB payload, 240.3 base64, 342.4 framing; about 328 MB of framing is WS server streams | 721.8, 239.3, 343.9 (whole chain at the seal, not the 16:40 observer line). WS server framing and base64: 362 MB |
| 4.5, 4.6 | Long window, synthetic: 3.6 times smaller, 14.5 to 37 to 1 | Real chunks: **4 times smaller, 12.9 to 1** at run 1's chunk sizes |
| 4.5 table | 168 h: 19 to 34 GB with the long window | About **17 GB** (low end) |
| 4.7, T8 | Cap backoff at 2 minutes; pass if no wait over 2 minutes | Jitter allows 150 s. Pass line: no wait over 2.5 minutes |
| 4.9, 6.3 G12 | The full trace was never scanned for secrets | Scanned on the server copy: 0 hits, all 378 auth headers masked (9.2) |
| 5.6 lesson 2 | HTTP POSTs were not attributed to threads | Every POST carries a `thread-id` header; per-thread matching works (9.4) |
| 8, first bullet | The server copy was not fetched | Fetched; producer, recovery and slot facts are now measured, not inferred |

### 9.9 What it adds to the fixes

| Fix | From | Size (est.) | When |
|---|---|---|---|
| Heartbeat says `failed` when the wire is off 60 s or the recorder is lost | 9.6 | about 10 src, 30 tests | Before Friday, with the count fix |
| Organizer query: requests rise, bytes per request collapse | 9.6 | SQL, no deploy | Run 2 and the event |
| A second email when `failed` follows `gaps` | 9.6 | about 8 SQL lines, 2 Worker lines, 20 tests | Owner's call |
| Verifier: `wire_ended_before_native`; wire error total | 9.5 | about 30 src, six goldens | Before Friday if green; else judge notes |
| Observer checks that fail run 1 on the bucket, liveness and native-coverage checks | 9.5 | organizer script | Before run 2 |
| Name the native limit and file count at the finish | 9.3 | about 3 src | With the native fix |
| Left guard on vendor-prefix masking rules | 9.2 | about 80 to 110 src | After Edition 01, unless run 2 shows a fail-closed span |
| Long-window compression | 9.7 | about 4 src | Before run 2, so run 2 measures it at the shipped chunk size |
| Upload pass line 2.5 minutes | 9.1 | text | Run 2 |
