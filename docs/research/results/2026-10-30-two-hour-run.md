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

## The ten measures

| # | Measure | Result | Evidence |
|---|---|---|---|
| 1 | Model calls | Trace: 842 calls (599 over WebSocket, 243 HTTP POSTs), plus 27 warm-up requests that generate nothing. Codex's record: 1,261. The 10-minute buckets match until 15:40. From 15:43:15Z the trace has 0 calls; Codex has 159, 140, 135 and 104 in its last four buckets. Every WebSocket call and response is the requested Luna model at low reasoning. The same holds for all 118 turn records in the 43 rollout files. The 243 HTTP bodies were not parsed (NOT VERIFIED there). | [final-reconcile.txt](2026-10-30-two-hour-run/final-reconcile.txt), [final.json](2026-10-30-two-hour-run/final.json) |
| 2 | Subagents | Codex started 42 (42 `spawn_agent` calls in the root, 42 child rollout files, all depth 1). Each file names its parent thread and has start and end times. The trace has wire calls for 28 of the 43 threads, plus 1 auxiliary thread that has no rollout file. The 15 subagents started at 15:48 or later have no wire call. 32 native streams were opened for 43 rollout files, so at least 11 files are probably missing natively. The full chain is pruned, so that part is NOT VERIFIED. | [final-reconcile.txt](2026-10-30-two-hour-run/final-reconcile.txt) |
| 3 | Hosts | Recorded: `api.openai.com` (`/v1/responses`, WebSocket and HTTP; `/v1/alpha/search` once) and `chatgpt.com` (`/backend-api/codex/models`, and `/backend-api/plugins/featured`, which got 401). Sealed: `registry.npmjs.org` x105 (80 MB), `ab.chatgpt.com` x11 (24 MB), `raw.githubusercontent.com` x1, `github.com` x1. All are `not_model_host`, with `dropped 0`. No model host is in the sealed list, yet model traffic after 15:43 was relayed and not recorded. | [tunnels.jsonl](2026-10-30-two-hour-run/tunnels.jsonl), [final-pane.txt](2026-10-30-two-hour-run/final-pane.txt) |
| 4 | Gaps | 23. 22 are `queue_full` (first at 15:15:12Z, last at 15:43:15Z) and 1 is `limit_exceeded` at the seal (16:54:43.81Z, no stream). 17 `recovery` records (`ingest_lost`), each moving to a new producer, all between 15:15 and 15:43. 14 streams ended `interrupted`. `host.log` shows no proxy restart (port rebind). | [final.json](2026-10-30-two-hour-run/final.json) `other`, [log.jsonl](2026-10-30-two-hour-run/log.jsonl) |
| 5 | Upload | 20 chunks stored, 1,304,968,017 bytes of chain, all acknowledged with the seal by 16:54:46Z (3 s after the seal). 11 network errors (os errors 11, 104 and 32, plus one EOF), with backoff up to 1,083 s. Lag: at 15:55 the chain was 1,302 MB but only 94 MB compressed had been sent; at 16:40, 276 MB. Local verify: exit 1, `chain_pruned_locally`, because goalcup pruned every part but the last when the seal was acknowledged. Server copy: **NOT VERIFIED**, because `fetch-run.sh` needs `DATABASE_URL` and Cloudflare R2 credentials, which this session does not have. | [host.log](2026-10-30-two-hour-run/host.log), [verify-local.txt](2026-10-30-two-hour-run/verify-local.txt) |
| 6 | Size | Chain: 1.30 GB and 479,319 records. Compressed upload: 405 MB (3.2 to 1). 1,302 MB came in the first 61 minutes, about 1.3 GB an hour while recording. Payload 722 MB: HTTP request bodies 375 MB (243 POSTs, each resends the full context), WebSocket client messages 158 MB, server messages 104 MB, rollouts 47 MB. The chain is 1.8 times the payload. 152,728 WebSocket server-message streams, each written as open, segment and end. | [log.jsonl](2026-10-30-two-hour-run/log.jsonl) |
| 7 | Load | Daemon: 50 to 52 MB RSS, 9 to 17% of a core, 17 to 18 open files. `goalcup start` (proxy and uploader): 26 to 32 MB, about 1%, 19 to 67 open files. Codex: 320 to 385 MB, 6 to 12%, up to 97 open files. Disk free stayed above 61 GB. | [log.jsonl](2026-10-30-two-hour-run/log.jsonl) `procs` |
| 8 | Secrets | No hit for the OpenAI key, the GitHub token or an `Authorization: Bearer` header in the 60 files still on disk. That covers the last chain part (1.4 MB, decoded segments too), all 43 rollouts (58.5 MB), the tunnel list, `host.log`, `daemon.log` and the upload state. **The full 1.3 GB trace was not scanned:** it was pruned at the seal, before the scan ran. Repository: nothing was pushed, so the GitHub repo is still empty. The 32 local commits do not contain the token. | [secrets-scan.txt](2026-10-30-two-hour-run/secrets-scan.txt), [secrets.py](2026-10-30-two-hour-run/secrets.py) |
| 9 | Harness experience | First call under 1 s after the prompt. Codex stopped twice and waited ("Goal achieved"). It never pushed: "no Git remote is configured". 95 calls failed with `rate_limit_exceeded` (2M tokens a minute) from 15:15 to about 15:43, while 20+ subagents ran; Codex retried and moved to HTTP POSTs. One startup warning: `-c` overrides force Codex's embedded mode. No refusal or question blocked it. | [final-pane.txt](2026-10-30-two-hour-run/final-pane.txt), [final.json](2026-10-30-two-hour-run/final.json) |
| 10 | Spend | Trace: 44.5M input tokens (44.0M cached), 223k output, about $0.61. Codex's record: 117.4M input (116.1M cached), 410.6k output (152k reasoning), about $1.50. Prices are third-party list prices ($0.10 input, $0.01 cached, $0.50 output per 1M tokens), not checked on OpenAI's own page. Well under the $20 cap. | [final.json](2026-10-30-two-hour-run/final.json) `usage` |

## Findings

1. **Silent loss after repeated `ingest_lost` (points 1, 2, 3).** Producers failed 17 times in 28 minutes.
   Each failure wrote a `queue_full` gap, then a `recovery` to a new producer. After the last gap at 15:43:15Z,
   no recovery and no record followed. The proxy kept relaying `api.openai.com` (Codex had no direct outside
   connection; `ss` showed every Codex socket on the loopback proxy). None of it was written. Likely cause: the
   sink was disabled for good once `MAX_CONNECTION_FAILURES` (10) was reached in a row
   (`crates/goalcup-cli/src/proxy/ingest.rs`). This is inferred from the code, not proven.
   Evidence: [final-reconcile.txt](2026-10-30-two-hour-run/final-reconcile.txt), [final.json](2026-10-30-two-hour-run/final.json).
2. **`queue_full` under a normal load (point 3).** 4 vCPU, about 20 subagents, under 1 model call a second, but
   about 52 WebSocket messages a second and up to 1.5 MB per request body. The daemon never used more than 17% of a
   core, so the bottleneck is probably the queue size or the socket, not the CPU.
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
| Never stop recording for good: after `ingest_lost`, keep reconnecting, and write one gap that covers the whole lost span with its end time | 1, 2, 3 |
| Find why the daemon queue fills at about 52 messages a second with 1.5 MB bodies, and size it for 40+ subagents | 1, 3 |
| Store a WebSocket connection as one stream per direction, not one per message; measure what repeated context costs the upload | 5 |
| Make the finish line and `goalcup status` say how long recording was lost, and count model calls one way | 6 |
| Name the `limit_exceeded` limit in the gap, or remove the limit | 2 |
| Keep the local trace until the participant verifies it, or make `verify:` point at the server copy | 5 |
| Upload in smaller chunks, more often, with a capped backoff | 5 |
| Put the push lines in the prompt for the participant, or tell them plainly at start that they are missing | 6, 7 |
