# Two-hour Codex run in a Vercel Sandbox, 29 Oct 2026 UTC (results)

A measured 2-hour organizer test run of Goal Cup Proxy on production. One agent acted as both the participant
and the observer. Evidence is in [2026-10-30-two-hour-run/](2026-10-30-two-hour-run/). In the evidence files,
the model name is replaced by `LUNA`. No key or token is in this file or the evidence.

## Setup

| What | Value |
|---|---|
| Build | `a31094c` (goalcup 0.7.0), built in a Claude Code container in 2 min 8 s, copied to `~/.local/bin` |
| Sandbox | `gc-2h-codex-b`, 4 vCPU, 8 GB, XFS, `iad1`, non-persistent, timeout raised from 3 h to 4 h |
| Server | production, `https://ingest.agentcommunity.org`, organizer test run |
| Run | `r632072599ee7b6fc`, entry `ede1ba278cd6261c8` |
| Harness | Codex 0.165.0 (the newest on npm), API-key login, the requested Luna model at low reasoning, `approval_policy = "never"`, `sandbox_mode = "danger-full-access"`, goalcup's time-left MCP tool. Subagents use the same model: every turn record says so |
| Repository | `nembal/gc-2h-codex-1030`, token in the project folder's `.env.local` (0600) |
| Prompt | typed 14:54:41Z, as written by the owner. Sealed 16:54:43Z |
| "keep going" | twice: 15:11:14Z and 16:43:55Z ([keepgoing.log](2026-10-30-two-hour-run/keepgoing.log)) |

Two deviations, both owner-approved or owner-caused:
- Stop rule: goalcup printed `PROBLEM: the recording has gaps` at about 15:15. The owner chose to keep going.
  The `no sleep inhibitor` PROBLEM line was there from the start. VERCEL-COMPAT already lists it as expected.
- Codex sat idle from 15:58 to 16:43, waiting for "keep going". The observer was blocked on the owner's answer,
  and its worker restarted. This idle time is not the harness's fault.

## Verdict

1. **Recording stopped silently under load.** From 15:43:15Z to the seal, the trace holds no model call at all.
   Codex's own record has at least 379 calls in that window, and about 470 counting the 15:40 bucket. That is about
   37% of the run's 1,261 calls, and 15 of 42 subagents. No new gap was written for this loss.
2. **Before 15:43, the wire matched Codex's own record** in every 10-minute bucket.
3. **The finish line hides the loss.** It said `Recording works: 1736 model call(s) recorded`. The status said
   only "the recording has gaps".
4. **The trace is large.** It grew to 1.30 GB in the 49 minutes it recorded. Most of it is full-context request
   bodies, and each WebSocket server message became its own stream.
5. **Upload worked, but lagged badly, and the local trace was pruned at the seal.** The full trace was never
   verified (see measure 5).

## Root cause (pinned after the run)

From the chain's gap and recovery times, and the code at `a31094c`:

1. **The daemon's queue filled.** The proxy's frames go to the daemon, which queues them for its one chain writer.
   That queue is 8 MiB, shared by every producer (`MAX_CAPTURE_QUEUE_BYTES`,
   `crates/goalcup-daemon/src/runtime/capture/sink.rs`). Request bodies averaged 1.5 MB, because each one resends
   the full context. About 20 subagents, plus retries after OpenAI's rate limits, sent bursts larger than the
   queue. The daemon then dropped that producer (`queue_full` gap) and closed its socket.
2. **The proxy counted each drop as a failure.** It saw a remote close (`ingest_lost`) and reconnected with a new
   producer (`recovery`). A connection that lives under 60 s (`STABLE_EPOCH`) does not reset the count
   (`crates/goalcup-cli/src/proxy/ingest.rs`, `lost` and `count_failure`).
3. **The 10th drop in a row turned recording off for good.** The connection that died at 15:39:17 had lived
   161 s, so the count restarted there. The next nine lived 3 to 45 s each. The 10th died at 15:43:15. That reached
   `MAX_CONNECTION_FAILURES = 10`, and `latch_terminal(IngestLost)` disabled the sink. Every later frame was
   dropped in the proxy, so the daemon never saw a loss to write as a gap. The relay went on, so the harness was
   never slowed.
4. **The finish line counted calls that were never written.** `record_generation_request` counts a request when the
   proxy selects it, not when it reaches the chain. It also counts every WebSocket client message, interrupts
   included. That is where "1736 recorded" comes from.
5. **The `limit_exceeded` gap at the seal is a separate limit.** It is the daemon's native-file finish marker:
   rollout files that never got a native stream slot (32 slots, 43 files; `owner.rs`, `uncaptured_files`).

So the "never stop" rule for a busy daemon is broken by a budget built for a daemon that is gone. A daemon that
drops a producer under load looks the same as one that is dead.

## The ten measures

| # | Measure | Result | Evidence |
|---|---|---|---|
| 1 | Model calls | Trace: 842 calls (599 over WebSocket, 243 HTTP POSTs), plus 27 warm-up requests that generate nothing. Codex's record: 1,261. The 10-minute buckets match until 15:40. From 15:43:15Z the trace has 0 calls; Codex has 159, 140, 135 and 104 in its last four buckets. Every WebSocket call and response is the requested Luna model at low reasoning. The same holds for all 118 turn records in the 43 rollout files. The 243 HTTP bodies were not parsed (NOT VERIFIED there). | [final-reconcile.txt](2026-10-30-two-hour-run/final-reconcile.txt), [final.json](2026-10-30-two-hour-run/final.json) |
| 2 | Subagents | Codex started 42 (42 `spawn_agent` calls in the root, 42 child rollout files, all depth 1). Each file names its parent thread and has start and end times. The trace has wire calls for 28 of the 43 threads, plus 1 auxiliary thread that has no rollout file. The 15 subagents started at 15:48 or later have no wire call. 32 native streams were opened for 43 rollout files, so at least 11 files are probably missing natively. The full chain is pruned, so that part is NOT VERIFIED. | [final-reconcile.txt](2026-10-30-two-hour-run/final-reconcile.txt) |
| 3 | Hosts | Recorded: `api.openai.com` (`/v1/responses`, WebSocket and HTTP; `/v1/alpha/search` once) and `chatgpt.com` (`/backend-api/codex/models`, and `/backend-api/plugins/featured`, which got 401). Sealed: `registry.npmjs.org` x105 (80 MB), `ab.chatgpt.com` x11 (24 MB), `raw.githubusercontent.com` x1, `github.com` x1. All are `not_model_host`, with `dropped 0`. No model host is in the sealed list, yet model traffic after 15:43 was relayed and not recorded. | [tunnels.jsonl](2026-10-30-two-hour-run/tunnels.jsonl), [final-pane.txt](2026-10-30-two-hour-run/final-pane.txt) |
| 4 | Gaps | 23. 22 are `queue_full` (first at 15:15:12Z, last at 15:43:15Z) and 1 is `limit_exceeded` at the seal (16:54:43.81Z, no stream). 17 `recovery` records (`ingest_lost`), each moving to a new producer, all between 15:15 and 15:43. 14 streams ended `interrupted`. `host.log` shows no proxy restart (port rebind). | [final.json](2026-10-30-two-hour-run/final.json) `other`, [log.jsonl](2026-10-30-two-hour-run/log.jsonl) |
| 5 | Upload | 20 chunks stored, 1,304,968,017 bytes of chain, all acknowledged with the seal by 16:54:46Z (3 s after the seal). 11 network errors (os errors 11, 104 and 32, plus one EOF), with backoff up to 1,083 s. Lag: at 15:55 the chain was 1,302 MB but only 94 MB compressed had been sent; at 16:40, 276 MB. Local verify: exit 1, `chain_pruned_locally`, because goalcup pruned every part but the last when the seal was acknowledged. Server copy: **NOT VERIFIED**, because `fetch-run.sh` needs `DATABASE_URL` and Cloudflare R2 credentials, which this session does not have. | [host.log](2026-10-30-two-hour-run/host.log), [verify-local.txt](2026-10-30-two-hour-run/verify-local.txt) |
| 6 | Size | Chain: 1.30 GB and 479,319 records. Compressed upload: 405 MB (3.2 to 1). 1,302 MB came in the first 61 minutes, about 1.3 GB an hour while recording. Payload 722 MB: HTTP request bodies 375 MB (243 POSTs, each resends the full context), WebSocket client messages 158 MB, server messages 104 MB, rollouts 47 MB. The chain is 1.8 times the payload. 152,728 WebSocket server-message streams, each written as open, segment and end. | [log.jsonl](2026-10-30-two-hour-run/log.jsonl) |
| 7 | Load | Daemon: 50 to 52 MB RSS, 9 to 17% of a core averaged over its life (`ps` gives no peak), 17 to 18 open files. `goalcup start` (proxy and uploader): 26 to 32 MB, about 1%, 19 to 67 open files. Codex: 320 to 385 MB, 6 to 12%, up to 97 open files. Disk free stayed above 61 GB. | [log.jsonl](2026-10-30-two-hour-run/log.jsonl) `procs` |
| 8 | Secrets | No hit for the OpenAI key, the GitHub token or an `Authorization: Bearer` header in the 60 files still on disk. That covers the last chain part (1.4 MB, decoded segments too), all 43 rollouts (58.5 MB), the tunnel list, `host.log`, `daemon.log` and the upload state. **The full 1.3 GB trace was not scanned:** it was pruned at the seal, before the scan ran. Repository: nothing was pushed, so the GitHub repo is still empty. The 32 local commits do not contain the token. | [secrets-scan.txt](2026-10-30-two-hour-run/secrets-scan.txt), [secrets.py](2026-10-30-two-hour-run/secrets.py) |
| 9 | Harness experience | First call under 1 s after the prompt. Codex stopped twice and waited ("Goal achieved"). It never pushed: "no Git remote is configured". 95 calls failed with `rate_limit_exceeded` (2M tokens a minute) from 15:15 to about 15:43, while 20+ subagents ran; Codex retried and moved to HTTP POSTs. One startup warning: `-c` overrides force Codex's embedded mode. No refusal or question blocked it. | [final-pane.txt](2026-10-30-two-hour-run/final-pane.txt), [final.json](2026-10-30-two-hour-run/final.json) |
| 10 | Spend | Trace: 44.5M input tokens (44.0M cached), 223k output, about $0.61. Codex's record: 117.4M input (116.1M cached), 410.6k output (152k reasoning), about $1.50. Prices are third-party list prices ($0.10 input, $0.01 cached, $0.50 output per 1M tokens), not checked on OpenAI's own page. Well under the $20 cap. | [final.json](2026-10-30-two-hour-run/final.json) `usage` |

## Findings

1. **Silent loss after repeated `ingest_lost` (points 1, 2, 3).** Producers failed 17 times in 28 minutes.
   Each failure wrote a `queue_full` gap, then a `recovery` to a new producer. After the last gap at 15:43:15Z,
   no recovery and no record followed. The proxy kept relaying `api.openai.com` (Codex had no direct outside
   connection; `ss` showed every Codex socket on the loopback proxy). None of it was written. Cause: see Root cause.
   Ten drops in a row, each under 60 s, reached `MAX_CONNECTION_FAILURES` and disabled the sink for good.
   Evidence: [final-reconcile.txt](2026-10-30-two-hour-run/final-reconcile.txt), [final.json](2026-10-30-two-hour-run/final.json).
2. **`queue_full` under a normal load (point 3).** 4 vCPU, about 20 subagents, under 1 model call a second, but
   about 52 WebSocket messages a second and 1.5 MB per request body on average. The daemon's lifetime CPU average
   was 17% of a core, but that hides bursts: whether its one writer was saturated is NOT VERIFIED.
3. **Trace size (point 5).** About 1.3 GB an hour while recording; a 168-hour run at this rate is about 200 GB.
   Two causes: every request body carries the whole context again (375 MB for 243 HTTP calls), and each of the
   152,728 WebSocket server messages is its own stream with three records.
4. **The finish line overstates (point 6).** `Recording works: 1736 model call(s)` matches neither the 842 calls in
   the trace nor the 1,261 in Codex's record, and it does not mention the hour of unrecorded calls. Evidence: [final-pane.txt](2026-10-30-two-hour-run/final-pane.txt).
5. **A `limit_exceeded` gap at the seal (point 2).** One gap with no stream at 16:54:43.81Z. It is probably a
   native-file or stream limit. Unknown which.
6. **Pruning at the seal removes the local evidence (point 5).** The `verify:` line printed at the finish points to
   a folder that can no longer verify whole. Only the server copy can, and that needs organizer credentials.
7. **Upload lag (point 5).** Chunks of up to 264 MB, with retries backing off to 18 minutes. At t+61 min only 7%
   of the chain (compressed) had reached the server. Heartbeats failed 3 times.
8. **No push (point 7).** goalcup expects its push lines at the start of the prompt. The owner's prompt did not
   have them, and the agent did not find another way. That is fair for the harness, but a participant who pastes a
   prompt will hit the same problem.

Observer notes: the observer script crashed once and was restarted 4 times. Each time it re-read the chain from the
start, so its totals cover the whole run. The 15:09 tick line was lost in a restart (that tick read 329 calls and
293 MB). The ticks at 15:27 and 15:28 come from a restart, not from the schedule.

## Fix first

| Fix | Point |
|---|---|
| Never stop recording for good: a daemon that answers the reconnect is alive, so a `queue_full` drop must not count toward `MAX_CONNECTION_FAILURES`; keep reconnecting, and write one gap that covers the lost span with its end time | 1, 2, 3 |
| Stop the daemon queue filling: 8 MiB shared by all producers is about five request bodies; size it for 40+ subagents, or apply backpressure per stream instead of dropping the whole producer | 1, 3 |
| Store a WebSocket connection as one stream per direction, not one per message; measure what repeated context costs the upload | 5 |
| Count a model call only when its frames reach the chain, and make the finish line and `goalcup status` say how long recording was off | 6 |
| Name the `limit_exceeded` limit in the gap, or remove the limit | 2 |
| Keep the local trace until the participant verifies it, or make `verify:` point at the server copy | 5 |
| Upload in smaller chunks, more often, with a capped backoff | 5 |
| Put the push lines in the prompt for the participant, or tell them plainly at start that they are missing | 6, 7 |

## Server copy

A read-only review of the copy the server holds, done on 29 Oct after the run. Inputs: the server copy, rebuilt with
`sh scripts/fetch-run.sh r632072599ee7b6fc <outdir>` (production bucket, the index query in a `READ ONLY`
transaction, R2 GETs only), and Codex's own record, `rollouts-r632072599ee7b6fc.tar.gz` (43 rollout files) from the
owner. The verifier was built at `a31094c`, the run's own build. Every check prints counts only. No key, token or
other secret is in this section. The model name is `LUNA`, as above.

### Whole chain

The server holds all of it. `fetch-run.sh` got 20 chunks, 1,304,968,017 bytes, seq 0 to 479,319, each chunk matching
its SHA-256 and its stated length. The offsets run on with no gap, and the last chunk meets the committed final
heartbeat. That is the same byte count the local finish reported, so the gap in measure 5 is closed. The upload lag
shows in the acks: the chunk acknowledged at 15:34:47Z ends at seq 341,077, and the next ack came at 16:26:11Z.

### 1. Verifier

| Mode | Exit | Result |
|---|---|---|
| `--capture-integrity` | 1 | `incomplete`, anchor `signed` (pinned production key), sealed, 479,320 records, 1,304,968,017 bytes, head `ede71518…`, 23 gaps, findings `gap_recorded`, `producer_recovered`, `producer_unclosed`. Ack verified through seq 479,319 (16:54:46Z), no torn tail, 0 records after the deadline. Terminal: `local_finish`, `manual`, termination `confirmed` by `process_signal` |
| `--research-json` | 1 | analysis `partial`, evidence `wire_and_native`, 34,036 lines. `cross_leg.state` is `consistent`, with the wire's last model request at 15:43:11.900Z and native activity to 16:54:43Z. Only `LUNA` is seen (123 native and 1,364 wire observations). `openai-responses` decoder: 8,726 observations, 429 unknown records, `incomplete` |
| `--stats` | 1 | 42 subagents spawned, 2 "keep going", 906 tool calls and 906 results. Tokens from native, `lower_bound` (only 32 of 43 files): 47.9M cache read, 432k input, 104k output (58k reasoning). Wire: 66.8M cache read, 664k input, 266k output, 675 responses (163 without usage). `provider_errors.total` is 0 |

No mode gave exit 2. Two blind spots: `cross_leg` says `consistent` although the wire stops 71 minutes before the
native leg ends, and `provider_errors` is 0 although the chain has about 97 rate-limit `error` events (see 4).
Only `producer_unclosed` hints at the loss.

### 2. Secrets

Hits: **0** for the OpenAI key and **0** for the GitHub token, in the whole 1.3 GB.

- **What was scanned.** Every stream's segments, base64-decoded and joined in `source_offset` order, so a secret split
  across two segments is still found: 374.0 MB of HTTP request bodies, 158.0 MB of WebSocket client messages,
  103.5 MB of server messages, 48.0 MB of native rollout streams, 36.2 MB of response bodies and 0.9 MB of HTTP
  headers. Also the open-record metadata (46.9 MB), the other records, the five side files (seal, baseline, receipt,
  acks, tunnel list) and the 43 raw rollout files (58.6 MB).
- **Compression.** No request or response in the trace has a `Content-Encoding` header, and no body starts with a gzip
  or zstd magic number. All 243 POST bodies were scanned as plain bytes. 19 streams were cut off by the producer drops
  (11 POST bodies, 8 WebSocket client messages); they were scanned as far as they were recorded.
- **How a hit is called.** Each whole token is extracted (`sk-[A-Za-z0-9_-]+`, `github_pat_[A-Za-z0-9_]+`, `gh[pousr]_…`)
  and the first 12 hex digits of its SHA-256 are compared with the prefixes the run's own scan recorded
  (`318af8b42a45`, `f30a3222026c`). There is no `sk-` token of 16 or more characters at a word boundary anywhere, and
  no GitHub-shaped token.
- **Positive control.** The scan found 378 `authorization` request headers. All 378 are `[masked]`. There is no
  unmasked `Authorization`, `Proxy-Authorization` or `x-api-key` value in any header, body or rollout. 22 other matches
  of `authorization` are a four-letter word in the `/backend-api/codex/models` response JSON, not a credential.
- **A side effect of redaction.** The daemon's redaction marks show 634 `openai_key` spans, 777 `huggingface_token`
  spans and 1 `github_token` span. In the raw rollouts, the same rules (`sk-(?:proj-)?[A-Za-z0-9_\-]{20,}`,
  `hf_[A-Za-z0-9]{20,}`) match 103 times. 102 matches are inside Codex's `encrypted_content` reasoning blobs, in the
  middle of a longer string. The `github_token` span (seq 328,018, a `response.completed` server message at
  15:16:32Z) is also in the middle of an `encrypted_content` blob; the rollouts have no GitHub-shaped string at all.
  None is a credential. The rules have no left word boundary, so the trace holds altered copies of those blobs.

### 3. When recording stopped

There is **no proxy-producer record after 15:43:15Z.** The last one is the `queue_full` gap at seq 478,379
(15:43:15.4427Z, producer `fd1a2b77`). The 940 records after it all come from the daemon: the native producer
`b22e4ea0` (to 16:54:43Z), the `limit_exceeded` gap, `producer_closed` and `local_finish`.

No record says the loss was final. No `recovery` follows the last gap. None of the 18 proxy producers has a
`producer_closed` record, which is why the verifier finds `producer_unclosed`.

The 22 `queue_full` gaps belong to the 18 proxy producers: one each, except `82ea14c8` (2), `bf0ec9e4` (2) and
`1ef9020a` (3). Each producer lived from its `recovery` record to its last gap:

| Producer | Lived | Gaps | Producer | Lived | Gaps |
|---|---|---|---|---|---|
| `76304176` (first) | 14:54:20 to 15:15:12, 20 min 51 s | 1 | `ee7b9797` | 15:36:36 to 15:39:17, 161 s | 1 |
| `e08c5c12` | to 15:18:50, 219 s | 1 | `bf0ec9e4` | to 15:39:29, 4 s | 2 |
| `efc1f8c8` | to 15:19:06, 15 s | 1 | `89dd6df4` | to 15:39:41, 3 s | 1 |
| `82ea14c8` | to 15:19:20, 9 s | 2 | `506f656b` | to 15:39:53, 4 s | 1 |
| `6941ee57` | to 15:25:01, 5 min 32 s | 1 | `1ef9020a` | to 15:40:33, 31 s | 3 |
| `f71b01c6` | to 15:26:36, 78 s | 1 | `df8a4019` | to 15:40:53, 12 s | 1 |
| `31b2cd13` | to 15:35:42, 8 min 44 s | 1 | `c6942b0d` | to 15:41:47, 45 s | 1 |
| `816cdaa5` | to 15:36:27, 36 s | 1 | `6d0d444c` | to 15:42:19, 23 s | 1 |
| | | | `50d8b30f` | to 15:42:50, 22 s | 1 |
| | | | `fd1a2b77` | to 15:43:15, 16 s | 1 |

That confirms the root cause above: after `ee7b9797` (161 s), nine producers in a row lived under 60 s, so the drop at
15:43:15 was the 10th failure in a row. Every gap has `lost_bytes: null`.

The `limit_exceeded` gap (seq 479,285, 16:54:43.810Z) has no stream. The daemon writes it at the native finish for
tracked files that never got a stream slot (`NativeFinishReport::uncaptured_files`, `native_capture.rs`). Native
capture keeps at most `MAX_OPEN_STREAMS / 4` = 32 streams open. All 32 native streams stayed open until the finish
(each ended `complete` at 16:54:43Z), so no slot was ever freed. The gap does not say which limit, or how many files.

**11 rollout files have no native stream**, as expected. The 32 captured files are the first 32 by start time (the
last one started at 15:56:43Z). The 11 missing are the subagents started from 16:44:01Z to 16:53:20Z: `f25ab829038f`,
`daa50ddd4334`, `bbeca6522167`, `94a721e8c5f9`, `e1ce19f4b6a0`, `767429f4ee4a`, `671ef80668b7`, `371d0177cc16`,
`8a0b1d40cd20`, `1c4cfb3a29f7`, `903f0d75bcfe` (last 12 hex digits of the thread id). They hold 136 of Codex's
`token_count` events. So for those 11 threads the trace has neither leg.

### 4. Model calls

Wire: **842 calls**, from 14:54:41.175Z to 15:43:13.555Z. That is 599 WebSocket `response.create` messages that
generate, plus 243 HTTP `POST /v1/responses` (232 parsed, 11 cut off and counted). Not counted: 27 warm-up
`response.create` with `generate: false`, and 37 `response.interrupt`. Codex: **1,261** `token_count` events with
usage, in 43 files.

| 10 min from | Codex | Wire (WS + HTTP) | Difference |
|---|---|---|---|
| 14:50 | 131 | 134 (134 + 0) | +3 |
| 15:00 | 193 | 192 (192 + 0) | −1 |
| 15:10 | 181 | 236 (169 + 67) | +55 |
| 15:20 | 105 | 104 (20 + 84) | −1 |
| 15:30 | 113 | 111 (60 + 51) | −2 |
| 15:40 | 159 | 65 (24 + 41) | −94 |
| 15:50 | 140 | 0 | −140 |
| 16:40 | 135 | 0 | −135 |
| 16:50 | 104 | 0 | −104 |
| **Total** | **1,261** | **842** | −419 |

- **Before 15:43:15.44Z:** Codex 788, wire 842. The wire's 54 extra are mostly in the 15:10 bucket, when OpenAI's rate
  limit hit. A refused attempt is a wire call with no `token_count`; the chain has about 97 `error` events with
  `rate_limit_exceeded` and 99 `response.failed` messages. From 15:40:00Z to the cutoff, both sides have exactly 65.
- **After 15:43:15.44Z:** Codex **473** (37.5% of 1,261), wire **0**. This replaces the "at least 379, about 470"
  estimate in the verdict.

Per thread, matched by the thread id in `client_metadata` (WebSocket) or the `thread-id` header (HTTP; all 243 POSTs
carry one): 28 of the 43 rollout threads have wire calls, and 1 wire-only thread (`…e2e4df5cb6a3`, 1 call) has no
rollout file. 21 threads match exactly. In 4 the wire has more, all retries (`6a2510a72d75` 65 against 55,
`a59924818ed7` 33/27, `5907d6356ab7` 45/23, `972cd4080400` 25/22). In 3 the wire has less: the root thread (358
against 591; before the cutoff Codex has 354, so all of the shortfall is after it), `15588487921b` (18/21) and
`3e03e40a451d` (3/21, started 15:42:55Z).
The 15 threads with no wire call are the 4 subagents started from 15:48:30Z to 15:56:43Z and the 11 from 16:44Z.

### 5. What the server shows

The server had the signal, but nothing on it shows the loss as a loss. From the database (read-only, no IP or member
columns read):

- **Run** `r632072599ee7b6fc`: mode `test`, `flagged_at` 15:29:25Z, `flag_reason` `gaps`. The run status is still
  `open`. That is by design: a segment sealed before `ends_at` leaves the run open (UPLOAD-API, seal).
- **Entry:** sealed 16:54:49Z, `upload_offset` 1,304,968,017, `final_seq` 479,319, `model_requests` **1,766**. That is
  the finish line's overcount, stored as the run's number.
- **Heartbeats:** 22. `recording` is `unknown` at 14:54:20Z, `ok` to 15:14:23Z, then `gaps` from 15:29:25Z to the
  final one. It is never `failed`, although the proxy's sink was off for good from 15:43:15Z. No heartbeat came from
  15:14:23Z to 15:29:25Z (two missed), or at about 15:49Z.
- **The tell the server holds and nobody reads:** from 15:44:27Z to the final heartbeat, `model_requests` rose by 600
  (1,166 to 1,766), while the chain grew by only 16.9 MB (`offset_local` 1,288,082,429 to 1,304,968,017), and that
  growth was all native. From 15:59:30Z to 16:39:33Z, nine heartbeats in a row have the same `seq`, 479,050.
- **Notices:** two, both sent: `started` (14:55:48Z) and `recording` (15:30:49Z, payload `recording: gaps`). That one
  email ("the tracer reports a recording problem (gaps)") is all the participant was told. No notice kind exists for
  "recording stopped".
- **Dashboard:** I found no page on the `agentCommunity_PAGE` main branch that reads the run's flag, heartbeats or
  notices. The Cloudflare dashboard has the Worker's logs only. So no screen shows the loss.

### Size and compression

The chain's 1,304,968,017 bytes split into 721.8 MB of payload (55%), 239.3 MB of base64 expansion (18%) and
343.9 MB of record framing (26%).

| Stream source | Streams | Chain | Payload |
|---|---|---|---|
| HTTP request (`wire_request`) | 758 | 506.3 MB | 375.4 MB |
| WebSocket server message (`ws_server`) | 152,728 | 465.7 MB | 103.5 MB |
| WebSocket client message (`ws_client`) | 671 | 214.3 MB | 158.2 MB |
| Native rollout (`native`) | 32 | 66.6 MB | 48.2 MB |
| HTTP response (`wire_response`) | 702 | 51.0 MB | 36.4 MB |

The WebSocket streams take 680.0 MB of chain for 261.7 MB of payload, so the per-message cost is 418.3 MB. For the
server messages alone it is 362 MB: 215.1 MB of open and end records, 112.3 MB of segment framing and 34.8 MB of
base64. The external review's estimate of about 466 MB is close to the whole `ws_server` share (465.7 MB); the
overhead measured here is 418 MB.

Recompressing each stored chunk's byte range with the uploader's level (`zstd -3`) gives 408.9 MB, which matches the
405 MB sent (3.2 to 1). With `zstd -3 --long=27` it is **100.8 MB (12.9 to 1), about 4 times smaller.** The gain
depends on the content: the 264 MB chunk of mostly WebSocket traffic goes from 8.7 to 11.4 to 1 (13.2 at `-19
--long=27`), and an 88 MB chunk of mostly HTTP bodies goes from 2.7 to 23.1 to 1. On this trace the result is at the
low end of the review's synthetic 14 to 37 times. A chunk compressed with `--long=27` needs a 128 MiB window when it
is decompressed. That cost on the server is NOT VERIFIED.

### What this adds to the fix list

| Fix | Why (from the server copy) |
|---|---|
| Send `recording: failed` when the sink is latched off, and add a notice for it | The server only ever saw `gaps`. The participant got one email at 15:30 and nothing at 15:43 |
| Flag a run whose `model_requests` rise while the chain does not grow | 600 calls counted against 16.9 MB of native-only growth after 15:44 |
| Make the verifier report a wire leg that ends long before the native leg, and count rate-limit errors | `cross_leg` said `consistent` and `provider_errors` said 0 |
| Give redaction rules a left word boundary | 102 of 103 rule matches in the rollouts, and the one `github_token` span, were inside `encrypted_content` blobs, now altered in the trace |
| Free a native stream slot when its file goes quiet, or raise the limit above the subagent count | All 32 slots stayed held to the end, so the last 11 subagents have no native record |
| Try `--long=27` in the uploader | 4 times less upload on this trace |
