# PRD-09: Usage and research stats

Id: PRD-09. Version: goalcup 1.0. Status: plan for the overnight build. Date: 30 October 2026.
Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md)
(section 15 is this PRD's contract). Checks that use it: [PRD-08](PRD-08-verifier-checks.md).

## Goal

For every model call in a run, say which model answered, how many tokens it used (input, cached,
output, reasoning), how it ended (ok, rate limited, error), and which agent or subagent made it. Sum
that per subagent and per run, so research can compare models and agents across runs.

## Points served

- **2** (every subagent): each call is tied to its thread, and each thread to its parent, from the
  record alone. A subagent with harness calls but no wire calls is named.
- **1** (every model call): the harness's own log is read back out of the record and joined to the
  wire, call by call. This join is what PRD-08's `wire_keeps_up` and `threads_match` read.
- **6** (tell the participant plainly): a short plain summary per run, ready for the participant if
  the owner says yes (PRD-00 question 6).
- **5** (rebuild the story by ID and time): every stat row points at an entry, a boot, a stream id
  and a wall time.

## Owner decisions it carries out

- Owner request (BRIEF3, "Added by the owner"): count tokens (input, cached, output, reasoning where
  given), find the model of every call, show per subagent which model it used and how many tokens and
  calls, in this build.
- **Record raw, interpret later stands** (`AGENTS.md:94-95`, D3 at `docs/DECISIONS.md:68`). The
  recorder never parses a body for this. The reader runs after upload, on an organizer machine.
- **Keep only the model and usage parts of today's decoders, cut the rest** (BRIEF3). Section 9 names
  the lines copied; PRD-13 deletes the old crate.
- **Token usage is derived, never enforced** (D26, `docs/DECISIONS.md:155-158`). No rule, cap or
  budget reads these numbers. Plan, credit and billing fields are never copied.
- One system for every harness (BRIEF3, 29 Oct night): one reader for every wire shape, gateways
  included. A gateway call is read exactly like a direct call; only its host and model names differ.
- Research value: as much as possible of what the agents did, every token any agent requested or
  produced (BRIEF3).
- The server stores bytes and interprets nothing. Usage JSON goes to R2, no database change
  (PRD-00 default "Usage reader", question 1; PRD-07 section 10).

## Today

| What | Where | Lines (src) | Fact |
|---|---|---:|---|
| Responses decoder (Codex HTTP and WebSocket) | `crates/goalcup-verifier/src/capture_analysis/openai.rs` | 664 | Model on request `:358-362` and on `response.created` `:427-436`; usage of the terminal event `:438-488` (fields `:117-119`); stream `error` `:544-560`; HTTP error body `:562-575` |
| Anthropic Messages decoder (Claude Code wire) | `.../capture_analysis/anthropic.rs` | 1,638 | Request model `:661-676`; `message_start` model and initial usage `:1121-1149`; `message_delta` cumulative usage `:1366-1380`; error type `:820-850`. Usage is "a cumulative snapshot, so nothing is ever summed" (`:33-35`) |
| Chat Completions decoder (Hermes, DeepSeek, gateways) | `.../capture_analysis/chat.rs` | 1,080 | Usage fields incl. DeepSeek's cache split `:128-137`; model, usage, error per chunk `:500-532`; request model `:984-989` |
| Amp thread decoder | `.../capture_analysis/thread.rs` | 383 | Usage snapshot `inputTokens`, `outputTokens`, `cacheCreationInputTokens`, `cacheReadInputTokens` and `usage.model` `:349-369`; subagent link `parentToolUseId` `:215-225` |
| Shared plumbing | `.../capture_analysis/wire.rs` | 1,339 | SSE framer `:458-660` (most of it handles v5 segments, base64 and redaction audits, all gone in GCR1); `put_usage` `:961-990` keeps integer members and names unknown ones |
| Native: Codex rollouts | `.../capture_analysis/native/codex.rs` | 443 | `turn_context` model `:85-97`; thread id and parent from `session_meta` `:117-180`; `token_count` `last_token_usage` and `total_token_usage` `:286-310`; `spawn_agent` with `receiver_thread_ids` `:400-430` |
| Native: Claude Code transcripts | `.../capture_analysis/native/claude_code.rs` | 219 | `message.model` and `message.usage` `:107-145`; an all-zero `<synthetic>` usage is skipped `:129-133` |
| Native: framework | `.../capture_analysis/native/mod.rs` | 617 | JSONL reassembly over v5 segments, caps (`MAX_OPEN_STREAMS`, 8 MiB shared buffer) |
| Run stats | `.../capture_analysis/stats.rs` | 1,115 | The normalisation table `usage_of` `:381-473`; "largest snapshot per key" `:317-341`; one error per call `:749-779`; spawn detection `:781-811`; Codex children left out `:515-520` |
| Token rules, written down | `docs/research/m2-report-format.md:221-238` | doc | One count per message id, response id or session; never sum cumulative usage; never add native and wire counts of the same call |
| Hermes, OpenCode, Kilo, Goose, Crush, OpenClaw SQLite | none | 0 | No decoder. Hermes subagent links exist only in `state.db` (`docs/plans/2026-10-30-audit/D-harnesses.md:993`, `:1095`) |

All of `crates/goalcup-verifier/src/capture_analysis/` is 13,109 lines with inline tests (`wc -l`).
Its decoder test files are about 11,000 lines (section 9).

**What today's tools said about run 1** (server copy review, `run-report-74daaa03.md`, sections 1
and 4, and measure 10):

| Source | Calls | Tokens |
|---|---|---|
| Wire, observer script (`final.json` `usage`) | 842 (599 WebSocket `response.create` that generate, 243 HTTP POSTs), plus 27 warm-ups and 37 interrupts not counted | 44.5M input of which 44.0M cached, 223k output, 77k reasoning |
| Wire, `goalcup-verify --stats` | 675 responses, 163 without usage | 664k input not cached, 66.8M cached, 266k output |
| Native, `--stats` | "lower bound", only 32 of 43 files | 432k input, 47.9M cached, 104k output, 58k reasoning |
| Codex's own record (all 43 rollout files from the owner's tarball) | 1,261 `token_count` events | 117.4M input of which 116.1M cached, 410.6k output, 152k reasoning |

Four tools, four answers, and no per-subagent view at all except a hand-made reconcile
(`final-reconcile.txt` on `origin/smart/tender-clarke-qupne3`). `provider_errors` said 0 while the
record held about 97 rate-limit errors. This PRD replaces all of it with one reader and one rule set.

## Design

### 1. Where it runs: an organizer job, after upload

| Option | Verdict | Why |
|---|---|---|
| Organizer job after upload | **Chosen** | Decoding needs the whole part with a 128 MiB window (PRD-01 section 15 point 9), so not in a Worker isolate (PRD-07 section 1). Reads the server copy, so the numbers are the server's, not a claim from the participant's machine. Rebuildable when the reader improves |
| Scheduled job | **Same job, on a timer** | The job runs every hour during the event on the organizer machine and picks up newly sealed entries. A run that is still open is read only on demand (`--live`), because each read downloads the whole entry |
| Client at finish, as a hint | **Rejected** | The recorder never parses (`AGENTS.md:94-95`). It would ship decoders in the participant's binary and slow the seal. Its numbers could not be checked. The participant's finish line keeps written counts and off time only (PRD-10) |

The same code runs locally on an entry folder: `goalcup-verify usage <entry dir>`. The pad (PRD-12)
and the watchers (PRD-16) use that, with no R2.

### 2. Shape of the reader

One streaming pass over the record (F1 `goalcup_format::read`, concatenated parts or fetched chunks),
then a join. New module `crates/goalcup-verify/src/usage/`:

| File | Does | Src (est.) |
|---|---|---:|
| `mod.rs` | `run(args)`, the U1 sink `Calls`, the join, totals, output | 250 |
| `wire.rs` | Pairs request and response streams; pairs WebSocket messages; picks the shape by path and body; status | 250 |
| `shapes.rs` | One small function per API shape: model, usage, error code. The normalisation table | 300 |
| `sse.rs` | A plain SSE line framer over decoded text (comments and keep-alives skipped) | 40 |
| `native.rs` | Session-file readers: Codex rollout, Claude Code transcript and `.meta.json`; a generic "file is a thread" rule for path-based harnesses | 250 |
| `sqlite.rs` | Rebuilds a final snapshot set (`x.db`, `-wal`, `-shm`), repairs WAL checksums, reads Hermes `state.db` | 150 |
| `hints.rs` | The per-harness table of where the thread hint lives (section 5), as one `match` on connector and row | 80 |
| `out.rs` | Usage JSON (S7), calls JSONL, research CSV, plain text summary | 150 |
| **Total** | | **about 1,470** (PRD-00 budget 1,500) |

Memory: per open stream a few words, plus one body at a time while it is parsed (Codex request
bodies averaged 1.5 MB in run 1). Per call one row of about 300 bytes. A busy week of about 100,000
calls is about 30 MB of rows (estimate). No caps of our own; this is an offline tool on an organizer
machine.

### 3. What a call is

A **call** is one model request attempt on the wire:

- an `http_req` stream whose method is `POST` and whose path ends in a known model path
  (`/responses`, `/messages`, `/chat/completions`, `:generateContent`, `:streamGenerateContent`),
  on any host. The path suffix, not the host, picks the shape, so gateway paths such as Cloudflare's
  `/v1/{account}/{gateway}/openai/responses` (`docs/plans/2026-10-30-audit/E-providers.md:350`) and
  Vercel's `https://ai-gateway.vercel.sh/v1/chat/completions`
  ([Vercel docs](https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-chat-completions)) are read
  like the provider's own;
- or one whole `ws_up` text message of type `response.create` that does not carry `generate: false`.

Not calls, but kept as rows with their own `kind`, because PRD-08's U1 needs them: `warmup`
(`response.create` with `generate: false`; 27 in run 1), `interrupt` (`response.interrupt`; 37 in
run 1), `other` (any other POST to a listed host, for example Codex's `/backend-api/codex/models`).

A **native call** is one entry in the harness's own log that says a model answered (section 4). The
two lists are joined in section 6. The COUNT event's "model requests written" (`http_post` +
`ws_up_msgs`, PRD-01 section 9) stays a request count; the call count comes from here.

**Pairing a request with its reply.**

- HTTP: the `http_resp` stream names its request in `req` (PRD-01 section 15 point 3). HTTP/1.1 and
  HTTP/2 alike.
- WebSocket: replies arrive on the connection's `ws_down` stream. A `response.create` is paired with
  the next `response.created` on the same connection. When both carry `previous_response_id`
  (`openai.rs:323-327, 414-418`), that id wins over order. Every server message up to the matching
  terminal event (`response.completed`, `response.incomplete`, `response.failed`) or a stream `error`
  belongs to that call.

**Status** (one per call, the owner's three plus two that keep the numbers honest):

| Status | When |
|---|---|
| `ok` | A terminal success: Responses `response.completed` or `response.incomplete` (the reason goes in `detail`); Anthropic `message_stop`; Chat a `finish_reason` or `[DONE]`; Gemini a final chunk with `finishReason` |
| `rate_limited` | HTTP 429, or an error code or type containing `rate_limit` (`rate_limit_exceeded`, `rate_limit_error`) in a `response.failed`, a stream `error` event or an error body |
| `error` | Any other HTTP status 400 and up, `response.failed`, an `error` event or error body. `detail` holds the HTTP status and the safe code token, for example `529 overloaded_error` |
| `cancelled` | The client interrupted it: a `response.interrupt` for it on `ws_up`, or the client closed the request before a terminal event |
| `unknown` | No terminal event in the record and no named cause: the reply stream ends `interrupted`, or a GAP covers its end. `whole: false` says why |

One status per call. A stream `error` event and a `response.failed` for the same call count once
(the rule of `stats.rs:749-779`, now true by construction). Run 1's server copy must read 95 to 99
`rate_limited` calls, not 0 and not about 196 (PRD-08 `provider_errors`).

Each row also carries **`whole`**: true when the request stream, the reply stream and, for
WebSocket, the message ended `complete` with no GAP over them. A call can be `ok` and not whole (the
request body had a gap but the reply arrived). Run 1 had 19 cut streams (11 POST bodies, 8
WebSocket client messages).

### 4. Model and tokens

**Model.** `model_requested` is the request body's `model`. `model` is what the reply says: the
Responses `response.model`, Anthropic `message_start.message.model`, the Chat chunk `model`, Gemini
`modelVersion` ([Gemini API](https://ai.google.dev/api/generate-content)), else the requested one.
Gateways often differ between the two (a `vendor/model` request and a dated reply name); both are
kept. Model names are kept as given, printable ASCII up to 128 bytes; a value that looks like a path
or URL is never shown as a model (`stats.rs` test at `:1073`). `effort` is kept where given
(Responses `reasoning.effort`, Codex `turn_context` effort).

**Tokens.** Five numbers per call, each `null` when the source did not say. Never 0 for "not said"
(appendix O, `O-harness-access.md:237, 2004`).

| Field | Meaning |
|---|---|
| `input` | Input tokens not read from a cache |
| `cached` | Input tokens read from a cache |
| `cache_write` | Input tokens written to a cache (Anthropic, some gateways) |
| `output` | Output tokens, reasoning included |
| `reasoning` | The reasoning part of `output`, where given |

The normalisation table (copied from `stats.rs:381-473`, plus Gemini and gateways):

| Shape | `input` | `cached` | `cache_write` | `output` | `reasoning` | Rule |
|---|---|---|---|---|---|---|
| Responses (`/responses`, HTTP and WebSocket) | `input_tokens` minus `input_tokens_details.cached_tokens` | `input_tokens_details.cached_tokens` | null | `output_tokens` | `output_tokens_details.reasoning_tokens` | Usage of the terminal event only (`openai.rs:474-481`) |
| Anthropic Messages | `input_tokens` | `cache_read_input_tokens` | `cache_creation_input_tokens` | `output_tokens` | null | `message_start` then every `message_delta`; per field the largest value wins, never a sum (`anthropic.rs:33-35`; `stats.rs:317-341`) |
| Chat Completions (DeepSeek, OpenRouter, Vercel AI Gateway, others) | `prompt_tokens` minus cached | `prompt_tokens_details.cached_tokens`, else `prompt_cache_hit_tokens` | `prompt_tokens_details.cache_write_tokens` | `completion_tokens` | `completion_tokens_details.reasoning_tokens` | The last `usage` seen. A stream with no usage chunk (no `include_usage`) is null. OpenRouter's extra usage chunk and second `finish_reason` are fine: last usage wins ([OpenRouter](https://openrouter.ai/docs/use-cases/usage-accounting); `E-providers.md:337`) |
| Gemini `generateContent` | `promptTokenCount` minus `cachedContentTokenCount` | `cachedContentTokenCount` | null | `candidatesTokenCount` plus `thoughtsTokenCount` | `thoughtsTokenCount` | Last `usageMetadata`. Shape from the [Gemini API page](https://ai.google.dev/api/generate-content); unverified on real traffic until the pad runs Gemini CLI |
| Amp thread snapshot | `inputTokens` | `cacheReadInputTokens` | `cacheCreationInputTokens` | `outputTokens` | null | Largest per `messageId` (`thread.rs:349-369`). A snapshot with no id is not counted |
| Codex `token_count` (native) | `last_token_usage.input_tokens` minus `cached_input_tokens` | `cached_input_tokens` | null | `output_tokens` | `reasoning_output_tokens` | One native call per event whose `total_token_usage` changed since the file's last event (a repeat with the same total is skipped and counted in `repeats_skipped`) |
| Claude Code transcript (native) | `message.usage.input_tokens` | `cache_read_input_tokens` | `cache_creation_input_tokens` | `output_tokens` | null | One per `message.id`; Claude Code repeats the usage on every content-block line (`m2-report-format.md:236`); largest wins. Model `<synthetic>` or all-zero usage is not a call (`claude_code.rs:129-133`) |
| Hermes `state.db` (native) | per session and model, from `session_model_usage` | same | same | same | same | Column names unverified on 0.22.5 (schema 32, `D-harnesses.md:964-968`); the builder reads them from the first pad sample. Per session totals only, no per call (`docs/harnesses/hermes-cli.md:62`) |

Other numeric usage members whose name ends in `tokens`, `Tokens` or `TokenCount` are kept per call
under `extra` (for example `audio_tokens`). Everything else in a usage object (`cost`,
`cost_details`, credits, plan, Codex `rate_limits`) is never copied (D26).

The **token source per call** is chosen once and named in `usage_from`: the wire usage when the call
has it; else the matched native call's usage; else null. Wire and native usage of the same call are
never added (`stats.rs:9-13`, contract C4). Run and subagent totals sum the calls, so each call
counts once whichever leg it came from. Both legs' own totals stay beside the merged one, for
research.

### 5. Threads and subagents: where each hint lives

The recorder adds no hint (PRD-01 section 5). This table is the reader's, in `hints.rs`. A harness
not in it, or a call with no hint, goes to the thread `unattributed`, said plainly.

| Harness | Wire thread hint | Native thread | Parent | Label | Status |
|---|---|---|---|---|---|
| Codex | WebSocket: the thread id inside `response.create` `client_metadata` (run 1 `measure.py:38`); HTTP: the `thread-id` request header, on all 243 POSTs of run 1 (`M-two-hour-run.md:946`) | Rollout file, `session_meta.payload.id` (`codex.rs:169-172`) | `payload.source.subagent.thread_spawn.parent_thread_id` (`codex.rs:123-128`) | `agent_path` and `depth` from the same `thread_spawn` (run 1 `final.py`) | Measured on run 1 |
| Claude Code | None needed: the join (section 6) gives the thread | The transcript file: `projects/<p>/<session>.jsonl` main, `<session>/subagents/agent-<id>.jsonl` children (`O-harness-access.md:580-586`) | The session file the `subagents/` folder sits under | `agentType` from `agent-<id>.meta.json` (`M-two-hour-run.md:617`) | Measured on fixtures; pad confirms |
| Hermes | Unverified: the pad looks for a session header or body field on the first run | `state.db` `sessions` row | `sessions.parent_session_id` (`D-harnesses.md:993`) | session title if present (unverified) | Unverified until the pad |
| Amp | `params.parentToolUseId` on thread WebSocket snapshots (`thread.rs:11-14, 215-225`) | none local | the `Task` call it names | none | Measured on one capture |
| Qwen Code | none known | `subagents/<sessionId>/agent-<id>.jsonl` (`O-harness-access.md:1673-1678`) | the session it sits under | `.meta.json` | Path only; usage not read in 1.0 |
| Kimi Code | none known | `agents/agent-<n>/wire.jsonl` (`O-harness-access.md:354-356`) | `agents/main` | folder name | Path only |
| Gemini CLI | none known | `chats/<parent id>/<sub id>.jsonl` (`O-harness-access.md:1207-1212`) | the folder name | none | Path only |
| Goose, OpenCode, Kilo, Crush, OpenClaw | none known | child session rows in the final SQLite snapshot (`O-harness-access.md:854, 1413-1418, 1577-1582, 1760`) | parent column (names unverified) | none | Thread list only after a pad sample |
| Copilot CLI, Factory Droid, Cline CLI | none known | layout unverified (`O-harness-access.md:1307-1314, 1848-1852, 400-420`) | unverified | none | `unattributed` until a pad sample |

Rule for adding a harness: a reader for a session format is written only from a real pad sample,
never from a guess. Until then its wire calls still get full per-call stats, under `unattributed`
threads, and `readable` says the harness log is not read (PRD-08 then skips `threads_match`, never
passes it).

### 6. The join: wire calls against the harness's own log

Per thread, two lists ordered by time: wire calls with status `ok`, and native calls. Times: wire
calls use the reply's end (the time of its last DATA, mapped to wall time by the nearest LINK, PRD-01
section 6); native calls use the record's own `timestamp` (Codex, Claude Code; same machine clock).

- **By id, where an id exists.** Claude Code: the native `requestId` equals the reply's `request-id`
  header (`M-two-hour-run.md:617-619`; header capture unverified until the pad). One to one.
- **By order and time otherwise** (Codex has no call id: `TokenCountEvent` has only `info` and
  `rate_limits`, `M-two-hour-run.md:616`). Each native call takes the earliest unmatched `ok` wire
  call of the same thread whose end lies between 120 s before and 5 s after the native time.
- Results per call: `matched`, `native_only` (the harness made it, the wire has no `ok` copy),
  `wire_only` (on the wire with no harness entry: retries the provider refused, warm-ups excluded,
  auxiliary threads such as run 1's one wire-only thread `...e2e4df5cb6a3`).

Wire above harness is normal (refused retries). Harness above wire is the run 1 shape. PRD-08 turns
these counts into pass, named or fail; this PRD only counts and dates them.

### 7. Output and storage

Per entry, the job writes to the R2 bucket `goalcup-usage` (PRD-07 section 10):

| Key | What | Size (est.) |
|---|---|---|
| `usage/{event}/{run}/{entry}.json` | Summary: reader version, connector and harness version, coverage, per thread, per model, totals (schema below) | 10 to 100 KB |
| `usage/{event}/{run}/{entry}.calls.jsonl` | One line per row (calls, warm-ups, interrupts, others, native calls) | about 0.5 MB for 2 hours; about 30 MB for a busy week |
| `usage/{event}/{run}/run.json` | The run rollup: entries merged, threads that span a `continue` merged by thread id | as the summary |

Summary schema (abridged; keys sorted, so the same input gives the same bytes):

```json
{"v":1,"reader":"1.0.0","run":"r…","entry":"e…","connector":"codex-cli","harness_version":"…",
 "readable":true,
 "coverage":{"wire_calls":842,"native_calls":1125,"matched":788,"native_only":337,"wire_only":54,
             "unattributed":0,"by_10min":[["2026-10-29T14:50Z",131,134]]},
 "status":{"ok":0,"rate_limited":0,"error":0,"cancelled":0,"unknown":0,"not_whole":19},
 "tokens":{"merged":{"input":0,"cached":0,"cache_write":null,"output":0,"reasoning":0},
           "wire":{...},"native":{...},"calls_without_usage":0},
 "models":[{"model":"LUNA","requested":["LUNA"],"calls":0,"threads":43,"tokens":{...}}],
 "threads":[{"id":"…00ca29f189cb","parent":null,"depth":0,"label":null,"models":{"LUNA":591},
             "effort":["low"],"wire_calls":358,"native_calls":591,"matched":354,
             "status":{...},"tokens":{...},"first":"…","last":"…","source":"rollout"}]}
```

(Values here are placeholders except where section 8 gives them.) A calls line:

```json
{"k":"call","entry":"e…","boot":0,"sid":412,"msg":3,"t0":"…","t1":"…","host":"api.openai.com",
 "host_kind":"provider","shape":"responses","via":"ws","model_requested":"LUNA","model":"LUNA",
 "effort":"low","status":"ok","detail":null,"whole":true,"thread":"…","join":"matched",
 "usage_from":"wire","input":0,"cached":0,"cache_write":null,"output":0,"reasoning":0}
```

`host_kind` (`provider`, `gateway`, `vendor`) comes from the shared host list (PRD-11). Nothing in
either file is text from the run: no prompt, message, tool name, path, URL or key. Thread ids, the
Codex `agent_path` and Claude Code `agentType` labels (bounded to 128 printable bytes), model names,
host names and error code tokens only.

**The organizer research table.** `goalcup-verify usage table <folder of usage JSON>` writes one CSV
row per thread per run: `event, run, connector, harness_version, thread, parent, depth, label,
models, effort, wire_calls, native_calls, matched, native_only, ok, rate_limited, error, cancelled,
unknown, input, cached, cache_write, output, reasoning, first, last`. `--calls` writes one row per
call instead. Organizers read it with any spreadsheet or notebook. No server table, no website
change (PRD-00 question 1).

**The participant summary.** `goalcup-verify usage --text` prints five plain lines. By default only
organizers see it (PRD-00 question 6). If the owner says yes, organizers send it after the seal; the
CLI never computes it.

### 8. Run 1, read the 1.0 way

Input: run 1's server copy, rewritten to GCR1 by PRD-01's test converter (PRD-01 acceptance 4). It
holds 32 of the 43 rollout files. Known numbers come from the run report and `final-reconcile.txt`;
"computed" means the reader's first run fills it in.

**Coverage.**

| | Value | From |
|---|---:|---|
| Wire calls | 842 (599 WebSocket, 243 HTTP), 14:54:41Z to 15:43:13Z | run report section 4 |
| Warm-ups, interrupts (not calls) | 27, 37 | same |
| `rate_limited` | 95 to 99, one per call | run report measure 9 and section 4; PRD-08 |
| Not whole | 19 (11 POST bodies, 8 WebSocket client messages) | run report "Secrets" |
| Native calls in the record | 1,125 in 32 files (1,261 minus the 136 in the 11 files with no native stream) | run report section 3 |
| Threads | 32 native threads, 28 with wire calls, 1 wire-only, `unattributed` 0 | run report section 4 |
| `native_only` after 15:43:15Z | 337 (473 harness calls after the cutoff, minus the 136 never captured) | derived; must match within the tolerance of PRD-08 |
| Before 15:43:15Z | harness 788, wire 842 | run report section 4 |

**Threads (first rows of the research table; harness calls from all 43 files, wire from the record).**

| Thread | Parent | Label | Model, effort | Harness calls | Wire calls | Note |
|---|---|---|---|---:|---:|---|
| `00ca29f189cb` | none | main | LUNA, low | 591 | 358 | 354 before the cutoff; the shortfall is all after it |
| `3073502a8fd2` | main | `/root/consumer_ui` | LUNA, low | 17 | 17 | matched |
| `6a2510a72d75` | main | `/root/api_foundation` | LUNA, low | 55 | 65 | wire above: retries |
| `3e03e40a451d` | main | `/root/admin_tests` | LUNA, low | 21 | 3 | started 15:42:55Z, cut |
| `1a6fabf307f2` | main | `/root/stage6_backend_review` | LUNA, low | 25 | 0 | after the cutoff |
| `f25ab829038f` | main | `/root/stage7_product_plan` | LUNA, low | 4 | 0 | no native stream either: in the record the reader sees only the parent's `spawn_agent` naming it |
| `...e2e4df5cb6a3` | unknown | none | LUNA | 0 | 1 | wire-only auxiliary thread |

Every thread used one model at one effort. The per-model view is one row: LUNA, 43 threads.

**Tokens.** Codex's own record, all 43 files: 117.4M input of which 116.1M cached, 410.6k output,
152k reasoning (run report measure 10). In the reader's terms that is `input` 1.3M, `cached` 116.1M,
`output` 410.6k, `reasoning` 152k. On the converted copy the merged total is lower by the 136 calls
in the 11 missing files (computed). The wire-only total is the wire's own number (computed; today's
two tools gave 44.5M and 67.5M of input for it, section "Today"). A 1.0 run with nothing lost would
show the merged total equal to Codex's own total, because every successful call has a
`token_count` and refused calls have no usage.

**Participant summary, run 1 as it was:**

```
Run r632072599ee7b6fc, 2 h 0 min. 1 main agent and 42 subagents.
Codex's own log: 1,261 model calls. Recorded on the wire: 842, up to 15:43 (95 refused by a rate limit).
Not recorded on the wire: 473 calls, 15:43 to 16:54 (1 h 11 min). Session files read: 32 of 43.
Model: LUNA in every agent. Tokens (Codex's own log): 117.4M input (116.1M from cache), 410.6k output.
These numbers are for research. They are not used to judge your run.
```

The same run under 1.0 with nothing lost would read "Recorded on the wire: 1,261 calls (and 95
refused). Not recorded: none."

### 9. What is copied from today's decoders

Copied as ideas and short code, rewritten over GCR1 records (decoded, de-chunked, masked bodies;
no base64, no segments, no redaction audits):

| From | Lines | Becomes |
|---|---|---|
| `openai.rs` | `:117-119`, `:333-336`, `:358-362`, `:406-488`, `:544-575` | `shapes::responses` (model, usage, terminal status, error code) |
| `anthropic.rs` | `:661-676`, `:832-850`, `:1121-1149`, `:1366-1380` | `shapes::anthropic` |
| `chat.rs` | `:128-137`, `:469-532`, `:984-989` | `shapes::chat` |
| `thread.rs` | `:215-225`, `:349-369` | `shapes::amp` |
| `wire.rs` | `put_usage` `:961-990`; the SSE blank-line rule from `:458-660` | `shapes::put_usage`, `sse.rs` (40 lines, not 200) |
| `stats.rs` | `:317-341`, `:367-473`, `:749-779` | the normalisation table, largest-wins, one error per call |
| `native/codex.rs` | `:85-97`, `:117-180` (identity and parent only), `:286-310`, `:400-430` | `native::codex` |
| `native/claude_code.rs` | `:107-145` (model and usage only) | `native::claude_code` |

Everything else in those files is cut: authorship and human markers, tool calls and results, text
and message projections, redaction audit projection, the `DecoderV1` framework (`api.rs`,
`registry.rs`, `session.rs`, `operations.rs`), `tracker.rs`, `report.rs`, the rest of `stats.rs`
(PRD-08 section 6 lists them). Fixtures copied, as decoded bodies, into
`tests/usage_fixtures/`: `m2-openai/codex-0.153.0-ws-run.jsonl` (Codex WebSocket),
`m2-anthropic/claude-code-2.1.283-e01.jsonl`, the four `m2-native` excerpts, each with its
`provenance.json`.

## Files it owns

Create:

- `crates/goalcup-verify/src/usage/{mod.rs, wire.rs, shapes.rs, sse.rs, native.rs, sqlite.rs,
  hints.rs, out.rs}`
- `crates/goalcup-verify/tests/usage_shapes.rs`, `usage_join.rs`, `usage_sqlite.rs`,
  `usage_run1.rs`, `usage_out.rs`
- `crates/goalcup-verify/tests/usage_fixtures/**` (ask 2 below)
- `scripts/usage-job.sh` (ask 2 below): list sealed entries with no usage object or an older
  `reader` version (read-only database query, as `fetch-run.sh` does), run `fetch-run.sh`, run
  `goalcup-verify usage`, put the three objects into `goalcup-usage` with a write token scoped to that
  bucket only, delete the fetched copy. About 60 lines.

Change: none outside these. Delete: none (PRD-13 deletes the old crate and its decoder tests).

Must not touch: `crates/goalcup-verify/Cargo.toml` and `src/main.rs` (PRD-08), the rest of
`crates/goalcup-verify/src/` (PRD-08), `crates/goalcup-format/` (PRD-01), `crates/goalcup/`
(recorder PRDs: the recorder never parses), `server/` and `scripts/fetch-run.sh` (PRD-07),
`connectors/**` (PRD-11), `scripts/pad/**` (PRD-12), docs outside this file (PRD-15).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 | F1 `read` over parts and chunks; section 15 guarantees; `start` event keys `connector`, `harness_version`, `server_time`, `hosts`; the run 1 converter (acceptance 4) for proof 6 | Used here |
| PRD-04 | Native `path` (`<row>:<relative path>`), `gen`; snapshot meta `form: final`, `harness_exited`, `size`, `mtime_ns`. WAL checksum repair before opening a snapshot is this PRD's job (PRD-04 section 6 item 6, `:285-287`) | Used here |
| PRD-05 | Masks are `*` of the same length, so JSON stays parseable; usage and rate-limit fields stay readable (`PRD-05:396`) | Used here |
| PRD-07 | `goalcup-usage` bucket and its read and write tokens (section 9 and 10); `delete-test-runs.sh` empties `usage/test/`; fetch-run output layout (PRD-08's S10) | Used here |
| PRD-08 | **Seam U1**, given here: `usage::Calls::new(&Start)`, `push(&Record)`, `finish() -> CallTable`. `CallRow { leg: Wire or Native, entry, boot, sid or path, msg, thread, parent, wall_start, wall_end, kind: Call, Warmup, Interrupt or Other, outcome: Ok, RateLimited{status, code}, Error{status, code}, Cancelled or Unknown, whole, model, join }`. `CallTable { rows, threads, readable, children: parent to named child threads }`. `Outcome::is_error()` is true for `RateLimited` and `Error`, which is PRD-08's "error". Also `usage::run(args) -> ExitCode` for the `usage` subcommand. Dependencies for `Cargo.toml`: `rusqlite` with the `bundled` feature, for SQLite snapshots | Both |
| PRD-10 | Nothing in 1.0: the CLI shows written counts and off time only. If the owner answers question 6 with yes, PRD-10 may later print a link to the summary | none |
| PRD-11 | `connectors/model-hosts.toml` gives each host a kind (`provider`, `gateway`, `vendor`); read at build time with `include_str!` for `host_kind`. Connector names and native row names, used by `hints.rs` | Used here (ask of PRD-11: a `kind` per host) |
| PRD-12 | S7 and U1: the pad check "wire calls match the harness's own log" reads `coverage` (through PRD-08's `wire_keeps_up` and `threads_match`). The pad's three cases each get a usage summary in `PAD-RESULTS.md` | Used there |
| PRD-13 | Deletes `crates/goalcup-verifier/` after the fixtures in section 9 are copied | Done there |
| PRD-14 | `cargo test -p goalcup-verify` covers `tests/usage_*.rs` in `gate fast` | Used there |
| PRD-15 | `docs/USAGE-STATS.md` (or a section of `docs/RECORD-FORMAT.md`): the schema, the token table of section 4, the hint table of section 5 | Written there |
| PRD-16 | Watchers run `goalcup-verify usage <entry>` after each pad run and the 2-hour run; the morning hand-off includes the per-thread table | Used there |

## What is cut

| Path | Lines (src / tests) | Fate |
|---|---:|---|
| `crates/goalcup-verifier/src/capture_analysis/{openai,anthropic,chat,thread,wire,tracker}.rs`, `native/*` | 5,883 + 1,279 / inline | Model, usage, error and thread parts copied (section 9). The rest cut (deleted by PRD-13) |
| `.../capture_analysis/stats.rs` | 1,115 / 873 (`tests/it/m2_run_stats.rs`) | Normalisation table, largest-wins and one-error-per-call copied. Cut: tool, human message, keep-going, spawn and id-budget logic, the 128-trace cap (`:39`) and the 1,000,000-id budget (`:43`) |
| Decoder tests: `tests/it/m2_anthropic_decoder.rs` 2,658, `m2_openai_decoder.rs` 2,149, `m2_chat_decoder.rs` 2,524, `m2_thread_decoder.rs` 625, `m2_native_observations.rs` 1,737, `m2_hermes_capture_only.rs` 395, `connector_samples.rs` 239, `harness_contract_golden.rs` 487 | 0 / 10,814 | Cut. About 600 lines of new tests replace them (acceptance below) |
| Fixtures `m2-openai`, `m2-anthropic`, `m2-native`, `connectors` | 404 KB | Copied as decoded bodies (section 9); the v5 wrappers go |

## Acceptance

### Real-run proofs (pad, PRD-12; Linux sandbox; watcher runs `goalcup-verify usage` on the entry)

1. **Codex, direct key, prompt asks for subagents.** Pass: `readable: true`; every native call is
   `matched` to an `ok` wire call of the same thread (`native_only` 0); every rollout thread is in
   `threads` with its parent and `agent_path`; each thread's model equals its `turn_context` model;
   for every matched call, wire `input`, `cached`, `output`, `reasoning` equal the native
   `last_token_usage` values (report any mismatch with its call; pass needs 100%, or a written
   explanation per mismatch in the night log); `unattributed` 0.
2. **Claude Code, direct key, one subagent.** Pass: every native `requestId` with non-zero usage is
   `matched` by `request-id` to one wire call with equal tokens; the subagent thread shows its
   `agentType` and the model it used; the main session and the subagent have separate rows.
3. **Hermes, DeepSeek key, one delegated child.** Pass: the `state.db` snapshot opens after the WAL
   repair; `threads` holds the main session and the child with its `parent_session_id`; per-session
   model and tokens are listed; wire Chat calls show usage, or `null` (never 0) when the stream had no
   usage chunk; the summary says which.
4. **Gateway case.** One core harness through a gateway (for example Claude Code with
   `ANTHROPIC_BASE_URL` to Vercel AI Gateway, as measured before in
   `docs/research/results/2026-10-vercel-gateway.md:53`). Pass: calls read with `host_kind:
   gateway`; `model_requested` and `model` both shown; tokens present; the same checks as proof 1 or 2.
5. **Dodger case.** Pass: no wire call is invented; native calls are listed; the summary says "not
   recorded on the wire" with the harness's count, never a total that looks whole.
6. **Run 1, converted.** The section 8 numbers: 842 calls, 27 warm-ups, 37 interrupts, 95 to 99
   `rate_limited`, 19 not whole, 1,125 native calls in 32 files, 28 threads with wire calls plus 1
   wire-only, `unattributed` 0, `native_only` 337 within PRD-08's tolerance, every per-thread wire
   count equal to the run report's per-thread table. Runs in under 5 minutes on the sandbox.

### Automated tests (`tests/usage_*.rs`, about 600 lines)

- `usage_shapes.rs`: each copied fixture gives the pinned model and five token numbers; a Chat stream
  with no usage gives nulls; an OpenRouter-style stream with a keep-alive comment and two
  `finish_reason`s gives one usage; an Anthropic message with three content-block lines counts once;
  a `<synthetic>` line is no call; `cost` is never copied.
- `usage_join.rs`: Codex order-and-time join with a retry the provider refused (wire above harness)
  and a cut window (harness above wire); Claude Code id join; one `error` event plus one
  `response.failed` is one `rate_limited` call.
- `usage_sqlite.rs`: a WAL-mode database whose `-wal` frame has 20 bytes turned to `*` opens with all
  frames after the repair, and loses those frames without it.
- `usage_run1.rs`: PRD-08's run 1 shape builder (Codex WebSocket and HTTP calls, then silence while
  rollouts go on): `native_only` counts the silent window exactly.
- `usage_out.rs`: the summary JSON of a small fixed input is byte for byte pinned (keys sorted).

## Risks and blind spots

- **Harness formats change.** Session formats are internal (Claude Code says so,
  `D-harnesses.md:276-278`; Codex `token_count` per response is measured, not documented,
  `M-two-hour-run.md:861`). If a reader finds zero native calls while the wire has calls, `readable`
  becomes false and PRD-08 skips, never passes. The pad's named versions are the only ones known good.
- **The Codex join is by time.** Two calls of one thread finishing within seconds could swap; totals
  stay right, a per-call token pair could be crossed. Thread and day totals are what research uses.
- **Child rollouts may hold copied parent records** (`stats.rs:515-520`, "unmeasured"). Run 1 argues
  against it: 21 children matched the wire exactly. Guard: in a child file, records before its own
  `session_meta` time are skipped and counted.
- **WebSocket pairing by order** assumes one response at a time per connection (unverified for Codex
  0.165). `previous_response_id` covers most cases; a mismatch shows as a status oddity, not a lost
  call.
- **Usage is what the provider or gateway says.** A gateway may count with another tokenizer
  (OpenRouter says "the model's native tokenizer"). Not billing; no prices in 1.0 (D26).
- **Hermes wire calls may stay `unattributed`** if no request carries a session id; per-subagent
  numbers then come from `state.db` only, as totals, not per call.
- **SQLite after masking.** A mask inside a page that changes a structure byte (not a text value)
  could still make SQLite refuse the page. Then the snapshot is listed as unreadable and the wire
  stands. The WAL repair follows the documented checksum
  ([SQLite file format, WAL](https://www.sqlite.org/fileformat2.html#walformat)).
- **Masked bytes in a usage number** would make that call's usage `null`. Unlikely: masking rules
  match key shapes, not digits. Masks inside `encrypted_content` (run 1 had 102) keep the JSON valid.
- **Labels are agent-written text** (Codex `agent_path`). Bounded and masked like everything else;
  shown to organizers only by default.
- **A week-long run makes a large calls file** (about 30 MB). Fine for R2 and a notebook; the
  summary stays small.
- **The job needs production read access** (database read, R2 traces read) and an R2 write token on
  `goalcup-usage`. Overnight agents have none (PRD-07 section 9): the job is proven on a local Worker
  and run for real by the owner in the morning.

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Where it runs | Organizer job after upload; hourly on the organizer machine during the event | Owner |
| Live entries | Read only on demand (`--live`) | Owner |
| Who sees the summary | Organizers only (PRD-00 question 6) | Owner |
| Join window | 120 s before to 5 s after the native time | PRD-09 after the first Codex pad run |
| Model name length kept | 128 bytes | PRD-09 |
| Storage | Three objects per entry and run in `goalcup-usage` | Owner (question 1, a database table later) |
| Native formats in 1.0 | Codex, Claude Code, Hermes; others from pad samples only | PRD-16 overnight |

## Builder notes

Order of work:

1. `shapes.rs` and `sse.rs` first, from the copied fixtures; they need no format crate. Pin each
   fixture's numbers in `usage_shapes.rs` (H+1.5).
2. `wire.rs` over F1 records: HTTP pairing by `req`, WebSocket pairing, status, `whole`. Use
   PRD-08's record builders in `tests/common/`.
3. `native.rs` for Codex and Claude Code; `hints.rs`.
4. The join and the U1 sink. **Publish U1 by H+4**: PRD-08's `wire_keeps_up` and `threads_match`
   wait on it. Stub first: a `CallTable` from wire rows only, native later.
5. `out.rs`: summary, calls JSONL, `table`, `--text`.
6. `sqlite.rs` (WAL repair, Hermes) as soon as the first Hermes pad run gives a `state.db`.
7. `scripts/usage-job.sh`, proven against the local Worker.
8. Other harnesses' readers, only from their pad samples.

Traps from the audit and the old code:

- Responses `input_tokens` includes cached tokens; `output_tokens` includes reasoning
  (`m2-report-format.md:229`). Subtract once, in one place.
- Anthropic `message_delta` usage is cumulative: largest wins, never add (`anthropic.rs:33-35`).
- Claude Code repeats usage on every content-block line of one message; key by `message.id`.
- Codex `total_token_usage` is a running total; `last_token_usage` is per call. Never add the two.
- The Responses stream has no `[DONE]`; `response.completed` ends it (`O-harness-access.md:239`).
- Chat Completions without `include_usage` has no usage: `null`, not 0 (`O-harness-access.md:237`).
- `generate: false` is a warm-up, `response.interrupt` is not a call (run report section 4).
- Run 1's "1736 recorded" counted every WebSocket client message at selection
  (BRIEF2). This reader counts calls from written bytes only, which is the point.
- Do not decide the shape by host. Path suffix and body decide; gateways reuse provider paths.
- Masked header values are `*` of the same length; never treat a `*` run as an id.

How a watcher proves it in a real run: after each pad run, `goalcup-verify usage <entry dir>
--out <dir>` and `--text`; compare `coverage.native_calls` with a plain count of `token_count`
lines (Codex) or distinct `requestId`s (Claude Code) on disk, and the per-thread table with the
harness's own files. After the 2-hour run, put the per-thread table and the summary in the night log.

## Asks of PRD-00 and PRD-01

PRD-00:

1. **Seams.** Widen S7 from one key to the three keys of section 7 (`{entry}.json`,
   `{entry}.calls.jsonl`, `run.json`), stable by H+4. Add U1 (section "Interfaces", PRD-08 row) to
   the seams table, owner PRD-09, used by 08 and 12, stable by H+4. U1 extends PRD-08's sketch with
   `RateLimited`, `Cancelled` and `whole`.
2. **Ownership.** Add to PRD-09: `crates/goalcup-verify/tests/usage_fixtures/**` (fixtures are not
   covered by `tests/usage_*.rs`) and the new `scripts/usage-job.sh` (no owner in the map today).
   Say who writes `docs/USAGE-STATS.md`: default PRD-15, from sections 4, 5 and 7 here.
3. **Question 6** stays, with this PRD's default: the summary text exists, organizers only, and the
   CLI never shows tokens. If the owner says yes, organizers send it; no client change.
4. **Size budget.** Usage about 1,470 src and about 600 tests: inside the 1,500 estimate.
5. **The core-set "done" item** should name proof 6 here (run 1 converted reads the known numbers):
   it is the cheapest proof that the call count is real.

PRD-01:

1. **Snapshot sets.** The three files of one SQLite copy (`x.db`, `-wal`, `-shm`) are separate
   `snapshot` streams with no shared key. Add `set` to the snapshot meta (for example the main file's
   `fid` and copy time) so the reader rebuilds files taken at the same moment. Without it the reader
   pairs by path stem and nearest `mtime_ns`, which can mix copies after a re-read.
2. **WebSocket message end time.** A DATA's time is when its first byte was read (section 3). For a
   multi-piece message the end time is the last piece's start, close enough for the 120 s join;
   say so in section 15 so nobody reads it as an exact end.
3. **Naming.** Section 9 calls `http_post` + `ws_up_msgs` "model requests written". It includes
   warm-ups, interrupts and non-model POSTs to listed hosts (run 1: 27 and 37 and the `/models`
   calls). Keep the count, but say in section 9 that it is not a call count and that PRD-09's
   `coverage.wire_calls` is, so PRD-10 and the heartbeat never call it "calls".
4. **The run 1 converter** (acceptance 4) must carry native streams and WebSocket messages with
   their opcodes, not only sizes, or proof 6 here cannot run. PRD-08 asks the same (its ask of
   PRD-01 7).
