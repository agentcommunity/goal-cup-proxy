# PRD-09: Usage and research stats

Id: PRD-09. Version: goalcup 1.0. Status: plan for the build, revision 2. Date: 30 October 2026.
Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md)
(section 15 is this PRD's contract). Checks that use it: [PRD-08](PRD-08-verifier-checks.md).
Revision 2 follows PRD-00 and PRD-01 revision 2 and the three reviews (section "Review answers" at
the end). The alignment pass against OWNER-DECISIONS.md is applied (section "Alignment answers").

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
  the owner says yes (PRD-00 question 1, old 4).
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
- **Token usage is derived, never enforced** (D26, `docs/DECISIONS.md:155-158`). No contest rule
  reads these numbers (D26). The one reader that acts on them is the pad's per-run spend check (ledger
  T6, PRD-16 section 9): every 5 minutes during every pad run and A1 to A3 it reads `usage --json` on
  the open local entry in the box and prices the totals with the pad's rates in `scripts/pad/pad.toml`
  (PRD-12 section 7). This PRD has no prices. Plan, credit and billing fields are never copied.
- One system for every harness (BRIEF3, 29 Oct night): one reader for every wire shape, gateways
  included. A gateway call is read exactly like a direct call; only its host and model names differ.
- Research value: as much as possible of what the agents did, every token any agent requested or
  produced (BRIEF3).
- The server stores bytes and interprets nothing. Usage JSON goes to R2, no database change
  (PRD-00 default "Usage reader", question 1, old 1 and 7; PRD-07 section 10).
- **Slices** (PRD-00 revision 2, "Build order"): `goalcup-verify usage <entry> --json` for Codex and
  Claude Code is in the core slice; Hermes and its `state.db` in the second slice; the organizer job,
  its bucket, the run rollup, the research CSV and the text summary in the third slice. The run 1
  converter is cut (PRD-00 "Not in this build"; review B N1).
- **Hermes and "wire calls match"** (PRD-00 done item 1, owner question 7, old 19): when `state.db`
  message rows carry times, the per-call join is used; else the totals form after the seal (per
  entry, PRD-08 section 3.1).
- **Builds, tests and runs in Vercel Sandboxes** (ledger B1). The reader, like every `goalcup-verify`
  command of the build, runs in a box; the conductor only fetches, copies and puts (section 1).

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
| Hermes, OpenCode, Kilo, Goose, Crush, OpenClaw SQLite | none | 0 | No decoder. Hermes subagent links exist only in `state.db` (`docs/plans/2026-10-30-audit/D-harnesses.md:993`, `:1095`). Hermes's own storage doc lists a `messages` table with `timestamp REAL NOT NULL` (Unix seconds), `role`, `finish_reason`, `token_count`, and `sessions` columns `parent_session_id`, `model`, `api_call_count`, `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `reasoning_tokens` ([session-storage.md at v0.22.5](https://raw.githubusercontent.com/NousResearch/hermes-agent/v0.22.5/website/docs/developer-guide/session-storage.md), fetched 30 Oct; the doc says the lists are abridged; unmeasured on a real 0.22.5 file) |

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
| Scheduled job | **Same job, on a timer** (third slice) | The job runs every hour during the event on the organizer machine and picks up newly sealed entries. An entry that is still open is read on demand only, because each read downloads the whole entry |
| Client at finish, as a hint | **Rejected** | The recorder never parses (`AGENTS.md:94-95`). It would ship decoders in the participant's binary and slow the seal. Its numbers could not be checked. The participant's finish line keeps written counts and off time only (PRD-10) |

The same code runs locally on an entry folder or a fetched copy (S13 layout): `goalcup-verify usage
<dir> --json --out <dir>`. That is the core-slice deliverable. The pad (PRD-12) and the watchers
(PRD-16) use it inside the run's pad box, with no R2: after each run, and every 5 minutes on the open
entry for the spend check (ledger T6, PRD-16 section 9). An open entry needs no special mode: F1
`read` stops at the last whole frame, and the summary says `"sealed": false`.

During the build the conductor does only the steps that need credentials: it lists sealed entries
(read-only database), runs `fetch-run`, copies the fetched folder into a build box, and puts the
three output files into `goalcup-usage-staging`. `goalcup-verify usage` itself runs in the build box,
which holds no credential (ledger B1, PRD-00 "Where the build runs"). The conductor never compiles or
runs the reader. The production job is the owner's to start, on an organizer machine.

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
| `sqlite.rs` | Rebuilds a database copy by `set` (`x.db`, `-wal`, `-shm`), newest first, repairs WAL checksums, reads Hermes `state.db` (`sessions`, assistant `messages` rows) | 150 |
| `hints.rs` | The per-harness table of where the thread hint lives (section 5), as one `match` on connector and row | 80 |
| `out.rs` | Usage JSON (S7) and calls JSONL (core slice); research CSV, run rollup and plain text summary (third slice) | 150 |
| **Total** | | **about 1,470** (PRD-00 budget 1,500) |

Memory: per open stream a few words, plus one body at a time while it is parsed (Codex request
bodies averaged 1.5 MB in run 1). Per call one row of about 300 bytes. A busy week of about 100,000
calls is about 30 MB of rows (estimate). No caps of our own; this is an offline tool on an organizer
machine.

### 3. What a call is

A **call** is one model request attempt on the wire:

- an `http_req` stream whose method is `POST` and whose path, with the query removed, ends in a
  known model path (`/responses`, `/messages`, `/chat/completions`, `:generateContent`,
  `:streamGenerateContent`), on any host. `meta.path` keeps the query (PRD-01 section 4), so
  `/v1/messages?beta=true` and `...:streamGenerateContent?alt=sse` are calls. The path suffix, not the host, picks the shape, so gateway paths such as Cloudflare's
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
(the rule of `stats.rs:749-779`, now true by construction). Run 1 had 95 to 99 rate-limited calls,
many sent as both events, and today's tools read 0 or about 196; `usage_run1.rs` plants that shape
and must read each call once (PRD-08 `provider_errors`).

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
| Codex `token_count` (native) | `last_token_usage.input_tokens` minus `cached_input_tokens` | `cached_input_tokens` | null | `output_tokens` | `reasoning_output_tokens` | One native call per event whose `total_token_usage` is larger than the largest total seen in that file (a repeat with the same or a smaller total is skipped and counted in `repeats_skipped`) |
| Claude Code transcript (native) | `message.usage.input_tokens` | `cache_read_input_tokens` | `cache_creation_input_tokens` | `output_tokens` | null | One per `message.id`; Claude Code repeats the usage on every content-block line (`m2-report-format.md:236`); largest wins. Model `<synthetic>` or all-zero usage is not a call (`claude_code.rs:129-133`) |
| Hermes `state.db` (native) | per session: `sessions.input_tokens` | `cache_read_tokens` | `cache_write_tokens` | `output_tokens` | `reasoning_tokens` | Per-session totals, never per call: a Hermes native call (one `messages` row with `role` `assistant`, section 6) carries no usage, so its call takes the wire usage. Model per session from `sessions.model` (`session_model_usage` when a session used more than one; its columns are not in the doc). Whether `input_tokens` already includes cached tokens is unverified: the builder decides from the first pad sample by comparing with the wire sum of the same session. Column names from the vendor doc (section "Today"), unmeasured on 0.22.5 (schema 32, `D-harnesses.md:964-968`) |

**Restarts repeat native bytes.** After a recorder restart the next boot's native stream may repeat
bytes already written. Native bytes are deduped by (`path`, `gen`, offset) across boots before
parsing (PRD-01 section 15 point 5). A Codex `token_count` counts only when its `total_token_usage` is
larger than the largest total seen in that file, so a replayed stretch adds no call.

Other numeric usage members whose name ends in `tokens`, `Tokens` or `TokenCount` are kept per call
under `extra` (for example `audio_tokens`). Everything else in a usage object (`cost`,
`cost_details`, credits, plan, Codex `rate_limits`) is never copied (D26).

The **token source per call** is chosen once and named in `usage_from`: the wire usage when the call
has it; else the matched native call's usage; else null. Wire and native usage of the same call are
never added (`stats.rs:9-13`, contract C4). Run and subagent totals sum the calls, so each call
counts once whichever leg it came from. Both legs' own totals stay beside the merged one, for
research.

### 5. Threads and subagents: where each hint lives

The recorder adds no hint (PRD-01 section 5). This table is the reader's, in `hints.rs`. Two named
places hold a wire call that no thread claims, so neither is ever hidden:

- **`unattributed`**: an `ok` wire call of a harness that has a wire hint (Codex today) whose request
  carries none. This should be 0; a count above 0 means the reader or the harness changed.
  `coverage.unattributed` counts these, and PRD-12 C5 and PRD-16 X8 check it is 0.
- **`wire_only`**: a wire call that the join (section 6) could not place, for a harness whose thread
  comes from the join (Claude Code, Hermes) or that has no reader yet. Claude Code's small side calls
  that no transcript names, refused retries and Hermes's compression calls land here.
  `coverage.wire_only` counts them, with their hosts and models. Not a failure.

| Harness | Wire thread hint | Native thread | Parent | Label | Status |
|---|---|---|---|---|---|
| Codex | WebSocket: the thread id inside `response.create` `client_metadata` (run 1 `measure.py:38`); HTTP: the `thread-id` request header, on all 243 POSTs of run 1 (`M-two-hour-run.md:946`) | Rollout file, `session_meta.payload.id` (`codex.rs:163-166`) | `payload.source.subagent.thread_spawn.parent_thread_id` (`codex.rs:123-128`) | `agent_path` and `depth` from the same `thread_spawn` (run 1 `final.py`) | Measured on run 1 |
| Claude Code | None needed: the join (section 6) gives the thread | The transcript file: `projects/<p>/<session>.jsonl` main, `<session>/subagents/agent-<id>.jsonl` children (`O-harness-access.md:580-586`) | The session file the `subagents/` folder sits under | `agentType` from `agent-<id>.meta.json` (`M-two-hour-run.md:617`) | Measured on fixtures; pad confirms |
| Hermes | None known; the join gives the thread (section 6). The pad looks for a session header or body field on the first run and, if found, adds it here | `state.db` `sessions` row; its assistant `messages` rows are the native calls | `sessions.parent_session_id` (`D-harnesses.md:993`) | none (`sessions.title` is text from the run) | Columns from the vendor doc; unmeasured until the pad (second slice) |
| Amp | none until a pad sample (Amp has no key this cycle, ledger T2) | none local | none | none | none until a pad sample (no reader: `readable: false`, `no_reader`) |
| Qwen Code | none known | `subagents/<sessionId>/agent-<id>.jsonl` (`O-harness-access.md:1673-1678`) | the session it sits under | `.meta.json` | Path only; usage not read in 1.0 |
| Kimi Code | none known | `agents/agent-<n>/wire.jsonl` (`O-harness-access.md:354-356`) | `agents/main` | folder name | Path only |
| Gemini CLI | none known | `chats/<parent id>/<sub id>.jsonl` (`O-harness-access.md:1207-1212`) | the folder name | none | Path only |
| Goose, OpenCode, Kilo, Crush, OpenClaw | none known | child session rows in the final SQLite snapshot (`O-harness-access.md:854, 1413-1418, 1577-1582, 1760`) | parent column (names unverified) | none | Thread list only after a pad sample |
| Copilot CLI, Factory Droid, Cline CLI | none known | layout unverified (`O-harness-access.md:1307-1314, 1848-1852, 400-420`) | unverified | none | `wire_only` until a pad sample (no reader: `readable: false`, `no_reader`) |

**Per-call native readers in 1.0: Codex, Claude Code and Hermes only.** Every other harness has
`readable: false` with the reason `no_reader`, so PRD-08 skips `wire_keeps_up` and `threads_match`
(never passes them). That is only the optional local log check: PRD-12 C1 rests on the record (no
unexplained gap and the history check, PRD-08 section 3.2), so a missing reader is never a reason for
"not verified" (ledger T4). Their
wire calls still get full per-call stats, under `wire_only`, and the path-only rows above still give a
thread list (who spawned whom) where the path says so.

**Entries after `continue`.** `goalcup continue` carries native cursors over (PRD-04), so an entry's
native stream may start at `from` above 0 with no `session_meta` or `turn_context`. Such a stream is
read with the run's earlier entries of the same `path` (the job fetches the whole run), so thread
id, parent and model come from the file's first lines. Without them, the Codex thread id is taken
from the rollout file name, and parent and model stay `null`.

Rule for adding a harness: a reader for a session format is written only from a real pad sample,
never from a guess. A sample becomes a fixture only as an excerpt under
`tests/usage_fixtures/<connector>/`, cut from the masked record, with `goalcup-verify secrets
--known <the build's known values>` at 0 hits (run in a build box) written into its
`provenance.json` before the commit, and named in the PR so reviewer A reads it by eye (AGENTS.md
hard rule on committed fixtures, point 4).

**Database copies** (PRD-04 section 6). An entry may hold several copies of one database: hourly
periodic copies and one at each seal, each a `set`. The reader tries the newest set first and goes
back one set at a time until one opens after the WAL repair. The summary names the set it used, its
time and `harness_exited`. Native calls after that copy's time are not known, so the join compares
only up to it (`native_until` in the summary); wire calls after it are not counted against the
harness. If no set opens, `readable: false` with the reason `snapshot_unreadable`, PRD-08 skips the
optional local log check, and PRD-12 C1 is judged from the record alone (ledger T4). The wire record
stands either way.

### 6. The join: wire calls against the harness's own log

Per thread, two lists ordered by time: wire calls with status `ok`, and native calls. Times: wire
calls use the reply's end (the time of its last DATA, mapped to wall time by the nearest LINK, PRD-01
section 6); native calls use the record's own time (Codex and Claude Code `timestamp`, Hermes
`messages.timestamp`; same machine clock). For a harness with no wire hint (Hermes), the wire list
is not split by thread first: the join runs over the whole entry, and a matched wire call takes the
native call's thread.

- **By id, where an id exists.** Claude Code: the native `requestId` equals the reply's `request-id`
  header (`M-two-hour-run.md:617-619`; header capture unverified until the pad). One to one. When a
  native call's `requestId` matches no reply (a gateway may send its own `request-id`), Claude Code
  falls back to the entry-wide time join used for Hermes, and the summary counts these in
  `coverage.joined_by_time`.
- **By order and time otherwise** (Codex has no call id: `TokenCountEvent` has only `info` and
  `rate_limits`, `M-two-hour-run.md:616`; Hermes rows have none either). Each native call takes the
  earliest unmatched `ok` wire call of the same thread (Hermes: of the entry, to a Chat or Messages
  path) whose end lies between 120 s before and 5 s after the native time.
- **Hermes rows.** A native call is one `messages` row with `role` `assistant` whose time lies inside
  the entry, in a session that started at or after the run's start (the first entry's start; the
  database holds every session of that home). A row soft-archived by compaction (`active` 0) still
  counts: the call happened. Each session's own `api_call_count` is kept beside the row count (U1
    `native_total`); when they differ, the summary says so, U1 gives `native_form: Totals`, and C1 uses
  the totals form of PRD-00 done item 1 (per entry: all `ok` and refused wire Chat or Messages calls
  of the entry against the sum of the entry's sessions' `api_call_count`, PRD-08 section 3.1). If the
  0.22.5 file has no usable `timestamp`, U1 gives `native_form: Totals` and the same comparison
  applies.
- Results per call: `matched`, `native_only` (the harness made it, the wire has no `ok` copy),
  `wire_only` (on the wire with no harness entry: retries the provider refused, warm-ups excluded,
  auxiliary threads such as run 1's one wire-only thread `...e2e4df5cb6a3`).
- What PRD-08 reads: per thread, native calls, `ok` wire calls and `native_only`, each with times.
  For Hermes and Claude Code the per-thread wire count is the matched count by construction, so
  `threads_match` there reduces to "`native_only` within tolerance", which is the question it asks.

Wire above harness is normal (refused retries). Harness above wire is the run 1 shape. PRD-08 turns
these counts into pass, named or fail; this PRD only counts and dates them.

### 7. Output and storage

`usage --json --out <dir>` writes the first two files below to a local folder (core slice). In the
third slice the job puts all three (made in a build box, put by the conductor, section 1) into the
R2 bucket `goalcup-usage` (`goalcup-usage-staging` during the build; PRD-07 section 10, PRD-00 S7):

| Key | What | Size (est.) |
|---|---|---|
| `usage/{event}/{run}/{entry}.json` | Summary: reader version, connector and harness version, coverage, per thread, per model, totals (schema below) | 10 to 100 KB |
| `usage/{event}/{run}/{entry}.calls.jsonl` | One line per row (calls, warm-ups, interrupts, others, native calls) | about 0.5 MB for 2 hours; about 30 MB for a busy week |
| `usage/{event}/{run}/run.json` | The run rollup: entries merged, threads that span a `continue` merged by thread id (third slice) | as the summary |

Summary schema (abridged; keys sorted, so the same input gives the same bytes):

```json
{"v":1,"reader":"1.0.0","run":"r…","entry":"e…","connector":"codex-cli","harness_version":"…",
 "sealed":true,"readable":true,"reason":null,"native_until":"…","db_set":null,
 "coverage":{"wire_calls":842,"native_calls":1125,"matched":788,"native_only":337,"wire_only":54,
             "unattributed":0,"joined_by_time":0,"by_10min":[["2026-10-29T14:50Z",131,134]]},
 "status":{"ok":0,"rate_limited":0,"error":0,"cancelled":0,"unknown":0,"not_whole":19,"fail_closed":0},
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

`reason` is null when `readable` is true, else `no_reader`, `snapshot_unreadable` or `no_native`
(a reader found zero native calls while the wire had calls). `db_set` names the database copy used
(section 5).

`host_kind` (`provider`, `gateway`, `vendor`, or `other` for participant and config hosts) is read
from the record: the `conn` stream's meta `kind`, else the `start` event's `hosts` list (PRD-01
sections 4 and 11). The reader embeds no host list, so a `--hosts` run is read with its own kinds. A
gateway call is read like any other: the path picks the shape, `host_kind` only labels it. Nothing in
either file is text from the run: no prompt, message, tool name, path, URL or key. Thread ids, the
Codex `agent_path` and Claude Code `agentType` labels (bounded to 128 printable bytes), model names,
host names and error code tokens only.

**The organizer research table** (third slice). `goalcup-verify usage table <folder of usage JSON>` writes one CSV
row per thread per run: `event, run, connector, harness_version, thread, parent, depth, label,
models, effort, wire_calls, native_calls, matched, native_only, ok, rate_limited, error, cancelled,
unknown, input, cached, cache_write, output, reasoning, first, last`. `--calls` writes one row per
call instead. Organizers read it with any spreadsheet or notebook. No server table, no website
change (PRD-00 question 1). Until it lands, the per-thread rows are in the summary JSON.

**The participant summary** (third slice). `goalcup-verify usage --text` prints five plain lines. By
default only organizers see it (PRD-00 question 1, old 4). If the owner says yes, organizers send it after
the seal; the CLI never computes it.

### 8. Run 1 as the target the reader must explain

The run 1 converter is cut (PRD-00 "Not in this build", PRD-01 "What is cut"), so the reader never
reads run 1's own bytes. Run 1's numbers below do three jobs instead: they set the shape of the
synthetic `usage_run1.rs` test (built on PRD-08's `check_run1_shape.rs` builders), they show what
the summary would have said, and they are the comparison for A1, the 2-hour Codex run on 1.0 with
run 1's prompt (PRD-16 X8). Known numbers come from the run report and `final-reconcile.txt`. LUNA
is the run report's stand-in for the model name (run report lines 5 and 129).

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
`output` 410.6k, `reasoning` 152k. Today's two wire tools gave 44.5M and 67.5M of input for the
wire leg alone (section "Today"); that disagreement is what one reader with one rule set ends. A 1.0
run with nothing lost shows the merged tokens of `matched` and `native_only` calls equal to Codex's
own total, because every successful call has a `token_count` and refused calls have no usage;
`wire_only` calls (such as an auxiliary thread) are listed apart. That is the A1 comparison.

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
| `wire.rs` | `put_usage` `:961-990`; the SSE blank-line rule from `:458-660` | `shapes::put_usage`, `sse.rs` (40 lines, not 200) |
| `stats.rs` | `:317-341`, `:367-473`, `:749-779` | the normalisation table, largest-wins, one error per call |
| `native/codex.rs` | `:85-97`, `:117-180` (identity and parent only), `:286-310`, `:400-430` | `native::codex` |
| `native/claude_code.rs` | `:107-145` (model and usage only) | `native::claude_code` |

Everything else in those files is cut: authorship and human markers, tool calls and results, text
and message projections, redaction audit projection, the `DecoderV1` framework (`api.rs`,
`registry.rs`, `session.rs`, `operations.rs`), `thread.rs` (no Amp reader until a pad sample),
`tracker.rs`, `report.rs`, the rest of `stats.rs`
(PRD-08 section 6 lists them). Fixtures copied, as decoded bodies, into
`tests/usage_fixtures/`: `m2-openai/codex-0.153.0-ws-run.jsonl` (Codex WebSocket),
`m2-anthropic/claude-code-2.1.283-e01.jsonl`, the four `m2-native` excerpts, each with its
`provenance.json`.

## Files it owns

Create:

- `crates/goalcup-verify/src/usage/{mod.rs, wire.rs, shapes.rs, sse.rs, native.rs, sqlite.rs,
  hints.rs, out.rs}`
- `crates/goalcup-verify/tests/usage_run1.rs` (the one usage test file, ledger B10, PRD-14 section 3)
- `crates/goalcup-verify/tests/usage_fixtures/**` (ownership given by PRD-00)
- `scripts/usage-job.sh` (third slice; ownership given by PRD-00): list sealed entries with no usage
  object or an older `reader` version (read-only database query, as `fetch-run.sh` does), run
  `fetch-run.sh`, decode, put the three objects into the usage bucket named by
  `GOALCUP_USAGE_BUCKET` (default `goalcup-usage`; `goalcup-usage-staging` in the build) with a write
  token scoped to that bucket only, delete the fetched copy. Fetch and put run on the credential host
  (the conductor during the build, an organizer machine in production). Decode runs in a box: with
  `--box <name>` (always during the build) the script `sandbox copy`s the fetched folder into that
  build box, runs `goalcup-verify usage` there with `sandbox exec` and copies the three files out; no
  credential enters the box (ledger B1). Without `--box`, on the organizer machine, it runs
  `goalcup-verify usage` locally. About 70 lines.

Change: none outside these. Delete: none (PRD-13 deletes the old crate and its decoder tests).
Dependencies (`rusqlite` with `bundled`, `zstd`) are declared by PRD-08 in
`crates/goalcup-verify/Cargo.toml` in phase 0 (PRD-00 "Smaller interface rulings").

Must not touch: `crates/goalcup-verify/Cargo.toml` and `src/main.rs` (PRD-08), the rest of
`crates/goalcup-verify/src/` (PRD-08), `crates/goalcup-format/` (PRD-01), `crates/goalcup/`
(recorder PRDs: the recorder never parses), `server/` and `scripts/fetch-run.sh` (PRD-07),
`connectors/**` (PRD-11), `scripts/pad/**` (PRD-12), docs outside this file (PRD-15).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 | F1 `read` over parts and chunks (stops at the last whole frame of an open part); section 15 guarantees; `start` event keys `connector`, `harness_version`, `server_time`, `hosts` with `kind`; `conn` meta `kind`; section 9's "model requests written is not a call count" | Used here |
| PRD-04 | Native `path` (`<source>:<relative path>`), `gen`, `from`; snapshot meta `form: final`, `set`, `harness_exited`, `size`, `mtime_ns`; hourly periodic copies plus the seal copy. WAL checksum repair before opening a set, and `readable: false` for a set that will not open, are this PRD's job (PRD-04 section 6 item 6) | Used here |
| PRD-05 | Masks are `*` of the same length, so JSON stays parseable; usage fields stay readable and Codex `rate_limits` bodies are not masked; a rate-limit header whose name matches the name rule stays masked, which this PRD does not need (PRD-05 section 7, its interfaces row for PRD-09, acceptance A3) | Used here |
| PRD-07 | `goalcup-usage` and `goalcup-usage-staging` buckets and their read and write tokens (sections 9 and 10, third slice); `delete-test-runs.sh` empties `usage/test/`; fetched layout S13 (`rec.gcr.zst`, `fetch.json`) | Used here |
| PRD-08 | **Seam U1**, given here: `usage::Calls::new()`, `push(&Record)`, `finish() -> CallTable`. The sink reads the `start` EVENT from the pushed records; no `Start` type is added to seam F1. `CallRow { leg: Wire or Native, entry, boot, sid or path, msg, thread, parent, wall_start, wall_end, kind: Call, Warmup, Interrupt or Other, outcome: Ok, RateLimited{status, code}, Error{status, code}, Cancelled or Unknown, whole, model, join }`. `CallTable { rows, threads, readable, children: parent to named child threads }`. `Outcome::is_error()` is true for `RateLimited` and `Error`, which is PRD-08's "error". `CallTable` also carries per thread `native_only` and `native_until`, and `readable` with its `reason`. `CallTable.native_form: PerCall, Totals or None` per connector (Hermes: `PerCall` when its assistant `messages` rows carry a usable `timestamp`, else `Totals`), and per thread `native_total` (Hermes: the session's `api_call_count`) beside the row count, as PRD-08 section 3.1 reads them. The summary's `joined_by_time` says how many Claude Code calls fell back to the time join. Also `usage::run(args) -> ExitCode` for the `usage` subcommand. PRD-08 adopts this U1 as is (PRD-00 consistency item 85) and declares `rusqlite` (bundled) and `zstd` in phase 0 | Both |
| PRD-10 | Nothing in 1.0: the CLI shows written counts and off time only. If the owner answers PRD-00 question 1 (old 4) with yes, PRD-10 may later print a link to the summary | none |
| PRD-11 | Host kinds reach this PRD through the record (`start` event, `conn` meta), never through an embedded `model-hosts.toml`. Connector names and native row names, used by `hints.rs` | Used here |
| PRD-12 | S7 and U1: C1 reads `coverage` and `readable` (through PRD-08's `wire_keeps_up` and `threads_match`); C5 reads tokens, threads and `unattributed` (0 means only "no Codex-style call lost its hint"; `wire_only` is reported, not failed). The ledger line per run after the run comes from the summary totals. The per-run spend check (ledger T6) runs `usage --json` on the open entry in the pad box every 5 minutes and prices the totals with the pad's rates; this PRD has no prices and no `--live` mode. Every `goalcup-verify` command runs in the pad box (PRD-12 section 8). X8 reads: every `ok` call has a model; tokens are present or the call is counted in `tokens.calls_without_usage` with its status. Refused calls have no usage by nature (for C5 too). New readers only from pad samples | Used there |
| PRD-13 | Deletes `crates/goalcup-verifier/` after the fixtures in section 9 are copied | Done there |
| PRD-14 | `cargo test -p goalcup-verify` covers `tests/usage_run1.rs` in `gate fast` | Used there |
| PRD-15 | A section of `docs/RECORD-FORMAT.md` (PRD-15's choice): the schema, the token table of section 4, the hint table of section 5; the usage job in `docs/OPERATING.md`. The history docs this PRD cites (`docs/research/m2-report-format.md`, `docs/research/results/2026-10-vercel-gateway.md`) are kept in this build (PRD-00 item 102) | Written there |
| PRD-16 | Watchers run `goalcup-verify usage <entry> --json` in the run's box after each pad run and A1, and every 5 minutes on the open entry of every pad run and of A1 to A3, for the T6 spend check (section 9); X8 compares A1's totals with run 1's (section 8). X8 reads: every `ok` call has a model; tokens are present or the call is counted in `tokens.calls_without_usage` with its status. Refused calls have no usage by nature; the end-of-build hand-off includes the per-thread table | Used there |

## What is cut

| Path | Lines (src / tests) | Fate |
|---|---:|---|
| `crates/goalcup-verifier/src/capture_analysis/{openai,anthropic,chat,thread,wire,tracker}.rs`, `native/*` | 5,704 + 1,279 / inline | Model, usage, error and thread parts of `openai`, `anthropic`, `chat`, `wire` and `native/*` copied (section 9); `thread.rs` and `tracker.rs` cut whole. The rest cut (deleted by PRD-13) |
| `.../capture_analysis/stats.rs` | 1,115 / 873 (`tests/it/m2_run_stats.rs`) | Normalisation table, largest-wins and one-error-per-call copied. Cut: tool, human message, keep-going, spawn and id-budget logic, the 128-trace cap (`:39`) and the 1,000,000-id budget (`:43`) |
| Decoder tests: `tests/it/m2_anthropic_decoder.rs` 2,658, `m2_openai_decoder.rs` 2,149, `m2_chat_decoder.rs` 2,524, `m2_thread_decoder.rs` 625, `m2_native_observations.rs` 1,737, `m2_hermes_capture_only.rs` 395, `connector_samples.rs` 239, `harness_contract_golden.rs` 487 | 0 / 10,814 | Cut. About 600 lines of new tests replace them (acceptance below) |
| Fixtures `m2-openai`, `m2-anthropic`, `m2-native`, `connectors` | 404 KB | Copied as decoded bodies (section 9); the v5 wrappers go |

## Acceptance

### Real-run proofs (pad, PRD-12; Linux sandbox; watcher runs `goalcup-verify usage` on the entry in the run's pad box)

1. **Codex, direct key, prompt asks for subagents.** Pass: `readable: true`; every native call is
   `matched` to an `ok` wire call of the same thread (`native_only` 0 outside named GAPs and the
   seal cut); every rollout thread is in
   `threads` with its parent and `agent_path`; each thread's model equals its `turn_context` model;
   per thread, the wire sums of `input`, `cached`, `output`, `reasoning` over matched calls equal
   the native `last_token_usage` sums over the same calls; per-call mismatches are listed, not
   failed, because the time join may swap two close calls of one thread (review B T7);
   `unattributed` 0.
2. **Claude Code, direct key, one subagent.** Pass: every native `requestId` with non-zero usage is
   `matched` by `request-id` to one wire call; equal tokens per thread over matched calls, per-call
   mismatches listed, not failed; the subagent thread shows its
   `agentType` and the model it used; the main session and the subagent have separate rows.
3. **Hermes, DeepSeek key, one delegated child** (second slice). Pass: the seal's `state.db` set
   opens after the WAL repair; `threads` holds the main session and the child with its
   `parent_session_id`; the assistant `messages` rows of each session are the native calls and each
   is `matched` to one `ok` wire Chat call (`native_only` within PRD-08's tolerance), or, if the rows
   carry no usable time, the entry's wire calls match the sum of `api_call_count` within PRD-08's
   tolerance (PRD-00 done item 1, PRD-08 section 3.1); per-session model and tokens are listed; wire Chat calls show usage, or `null` (never 0) when
   the stream had no usage chunk; the summary says which. The first run also records, in the night
   log, the real column names and whether `sessions.input_tokens` includes cached tokens.
4. **Gateway case.** Each core harness through Vercel AI Gateway, the way PRD-12 sets it (for example
   Claude Code with `ANTHROPIC_BASE_URL`, as measured before in
   `docs/research/results/2026-10-vercel-gateway.md:53`). Pass: calls read with `host_kind:
   gateway`; `model_requested` and `model` both shown; tokens present; the same checks as proof 1, 2
   or 3, with Claude Code matched by id or, when the gateway rewrites `request-id`, by time; the
   summary says which (`joined_by_time`). No code path differs from the direct case except the host
   and model names.
5. **Dodger case.** The simulated dodger (PRD-12 section 2: a core harness with its model host
   marked `pinned`). Pass: no wire call is invented; native calls are listed; the summary says "not
   recorded on the wire" with the harness's count, never a total that looks whole.
6. **A1, the 2-hour Codex run on 1.0** (PRD-16; replaces the cut run 1 converter). Pass: the
   reader runs on the server copy in a build box; its time is logged, not a pass line; `native_only`
   within
   PRD-08's tolerance outside named GAPs; every rollout thread present with parent and label;
   `rate_limited` counted once per call; the merged tokens of `matched` and `native_only` calls equal
   the watcher's sum of `last_token_usage` over every rollout file, and `wire_only` tokens are listed
   apart; the per-thread table is compared
   with run 1's (section 8) in the hand-off.

### Automated tests (`tests/usage_run1.rs`, about 200 lines)

Few on purpose (ledger B10, PRD-14 section 3). Usage stats are proved by PRD-12 C5 on real runs
through the rig: model, tokens and status per call and per subagent, for every harness and case. One
test file guards what a real run cannot show every time. No test pins the output layout, a key order
or a field name. The shape, join, SQLite repair and output cases below are what the reader must do;
the rig's real runs and the watcher's count (below) prove them, not separate test files.

- `usage_run1.rs`, the only file: PRD-08's run 1 shape builder (Codex WebSocket and HTTP calls,
  warm-ups, interrupts, rate-limit errors sent both as `error` and `response.failed`, then silence
  while rollouts go on): `native_only` counts the silent window exactly; warm-ups and interrupts are
  not calls; each rate-limited call counts once; totals equal the sum of the rows; no output field
  holds text from a body, and a panic inside `usage` prints no record text (point 4, PRD-17 VR2).

What the reader must do, proved by the rig's real runs (C5) and the watcher's count, not by test
files (`usage_shapes.rs`, `usage_join.rs`, `usage_sqlite.rs` and `usage_out.rs` are cut, ledger B10):

- Shapes: each copied fixture gives the pinned model and five token numbers (checked once in the build
  box, builder note 1); a Chat stream with no usage gives nulls; an OpenRouter-style stream with a
  keep-alive comment and two `finish_reason`s gives one usage; an Anthropic message with three
  content-block lines counts once; a `<synthetic>` line is no call; `cost` is never copied;
  `/v1/messages?beta=true` and `:streamGenerateContent?alt=sse` are calls.
- Joins: Codex order-and-time join with a retry the provider refused (wire above harness) and a cut
  window (harness above wire); Claude Code id join, with a side call no transcript names landing in
  `wire_only`, not `unattributed`, and a reply whose `request-id` matches no transcript joined by time
  and counted in `joined_by_time`; Hermes entry-wide join of two parallel sessions; a Codex rollout
  whose last 10 lines repeat after a restart gives the same native calls as without the repeat.
- SQLite: a `state.db` whose `-wal` frame was masked opens with all frames after the repair; with two
  sets where the newer is broken, the older set is used and named; with only a broken set `readable`
  is false with `snapshot_unreadable`.
- Output: the same input gives the same output twice.

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
- **The Hermes join is entry-wide by time.** Up to 10 children run at once in one process (appendix
  O, H3), so two sessions' calls can finish within seconds and swap. Per-session call counts and
  totals stay right; a per-call pair can cross. If `messages.timestamp` turns out to be the time a
  row was saved rather than the reply's end, the 5 s window may be too tight; the first pad run sets
  it (overridable default). If the columns differ on 0.22.5, the totals form applies (PRD-08
  section 3.1).
- **A database copy is up to an hour old during the run.** `native_until` says how far the native
  leg reaches; a dead entry with no seal copy keeps only its last periodic copy (PRD-04 section 6
  item 8).
- **Fail-closed masking** (review A finding 3). A fail-closed span is cut at each WebSocket message
  end (PRD-00 S2 `boundary()`), so at most one message's body is starred; that call's model and usage
  may read `null`. The summary counts calls whose bodies hold a `fail_closed` mask, so a starred
  stretch is visible, not a silent drop in tokens.
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
- **The job needs read access** (database read, R2 traces read) and an R2 write token on the usage
  bucket. During the build these are on the conductor only, for staging (PRD-00 "Where the build
  runs"): the conductor lists, fetches and puts; `goalcup-verify usage` runs in a build box with
  `sandbox copy` and `sandbox exec`, and the job is proven that way against
  `goalcup-usage-staging` (ledger B1). A sandbox never holds the credentials. The owner runs it on
  production, on an organizer machine.
- **The spend check reads an open entry.** It reads up to the last whole frame, so it errs low by at
  most one frame; the pad's rates err high (PRD-16 section 9).

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Where it runs | Organizer job after upload; hourly on the organizer machine during the event (PRD-00 question 1, old 7) | Owner |
| Open entries | Read on demand, up to the last whole frame; `sealed: false` | Owner |
| Who sees the summary | Organizers only (PRD-00 question 1, old 4) | Owner |
| Join window | 120 s before to 5 s after the native time | PRD-09 after the first Codex pad run; Hermes after its first pad run |
| Hermes "wire calls match" | Per-call join on `messages` rows; totals form on `api_call_count`, per entry (PRD-00 question 7, old 19; PRD-08 section 3.1) | Owner |
| Token equality in proof 1 | Per-thread sums equal; per-call mismatches reported only | PRD-09 |
| Model name length kept | 128 bytes | PRD-09 |
| Storage | Three objects per entry and run in `goalcup-usage` (`-staging` in the build) | Owner (question 1, a database table later) |
| Native formats in 1.0 | Codex, Claude Code, Hermes; others from pad samples only | PRD-16 during the build |

## Builder notes

Order of work:

Core slice (start at H+0, review B O5):

1. `shapes.rs` and `sse.rs` first, from the copied fixtures; they need no format crate. Check each
   fixture's numbers once in the build box and log them (H+1.5); no `usage_shapes.rs` (ledger B10).
2. `wire.rs` over F1 records: HTTP pairing by `req`, WebSocket pairing, status, `whole`. Use
   PRD-08's record builders in `tests/common/`.
3. `native.rs` for Codex and Claude Code; `hints.rs`.
4. The join and the U1 sink. **Publish U1 by H+4**: PRD-08's `wire_keeps_up` and `threads_match`
   wait on it. Stub first: a `CallTable` from wire rows only, native later.
5. `out.rs`: summary and calls JSONL (`usage --json --out`).

Second slice:

6. `sqlite.rs` (WAL repair, newest readable set, Hermes `sessions` and `messages`). Write it from a
   WAL database made in the test; confirm the column names on the first Hermes pad `state.db`.

Third slice:

7. `run.json`, `table`, `--text`; `scripts/usage-job.sh`, proven against staging and
   `goalcup-usage-staging`: the conductor fetches and puts, the reader runs in a build box (section 1,
   ledger B1).
8. Other harnesses' readers, only from their pad samples.

Before writing `native.rs`, `sqlite.rs` or a shape, read the vendor's current docs and cite the page
in the commit (ledger T7).

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
  One reader for every connection: a direct key, Vercel AI Gateway, OpenRouter or Cloudflare AI
  Gateway differ only in host, path prefix and model names. Never add a per-gateway branch; a new
  gateway that reuses a known API shape needs no code.
- Masked header values are `*` of the same length; never treat a `*` run as an id.

How a watcher proves it in a real run: after each pad run, in the run's pad box, `goalcup-verify
usage <entry dir> --json --out <dir>`; compare `coverage.native_calls` with a plain count of
`token_count` lines whose total grew past the largest so far (Codex), distinct `requestId`s (Claude Code) or `sqlite3` counts of assistant `messages` rows
per session (Hermes) on disk, and the per-thread rows with the harness's own files. After A1, put
the per-thread table and the summary in the night log.

## Asks of PRD-00 and PRD-01 (status after their revision 2)

Every ask in this table is ruled; it is history, kept here until the review sections move to
`REVIEW-HISTORY.md` (alignment X2-11). The body above wins.

| Ask | Status |
|---|---|
| PRD-00 1: S7 widened to three keys, U1 in the seams table | Done (PRD-00 seams S7 and U1) |
| PRD-00 2: ownership of `usage_fixtures/` and `scripts/usage-job.sh`; who writes the usage docs | Done (PRD-00 ownership; PRD-15 writes a section of `docs/RECORD-FORMAT.md`) |
| PRD-00 3: question on who sees the summary | Done (question 1, old 4: organizers only) |
| PRD-00 4: size budget | Done (1,470 src, about 600 tests) |
| PRD-00 5: name run 1 converted in done item 1 | Withdrawn: the converter is cut. Proof 6 is now A1 |
| PRD-01 1: `set` on database copies | Done (PRD-01 section 4; PRD-04 writes it) |
| PRD-01 2: WebSocket message end time is approximate | Done (PRD-01 section 15 item 4) |
| PRD-01 3: "model requests written" is not a call count | Done (PRD-01 section 9) |
| PRD-01 4: run 1 converter keeps native streams and opcodes | Withdrawn: the converter is cut |

New asks:

1. **PRD-12 and PRD-16**: C5 and X8 read `unattributed` 0 with this PRD's meaning (section 5): only
   a Codex-style call that lost its hint. Claude Code side calls and Hermes compression calls are
   `wire_only`, reported, not failed.
2. **PRD-12**: Hermes's local log check needs the second slice's `sqlite.rs`; until it lands, that
   check is `skip` and Hermes C1 is judged from the record (ledger T4), never FAIL for the missing reader.
3. **PRD-08**: U1 now carries per-thread `native_only`, `native_until` and `readable` with `reason`,
   plus `native_form` and per-thread `native_total` (section "Interfaces"); `threads_match` for a
   harness whose thread comes from the join reads `native_only` per thread. PRD-08 section 3.1
   already reads it this way.
4. **PRD-08**: R5 passes with `harness_log` `per_call` or `totals`, whichever U1 gives (section 3.1
   already says so; R5 still names `totals` only).
5. **PRD-16**: X8 reads: every `ok` call has a model; tokens are present or the call is counted in
   `tokens.calls_without_usage` with its status. Refused calls have no usage by nature. PRD-12 C5
   reads tokens the same way.

## Review answers

This section is history. The body above, the ledger and the "Alignment answers" below win over it;
where an answer here disagrees with the body (for example CC 19 on where the job runs, B N4 on
pricing), the body is right. It moves to `REVIEW-HISTORY.md` in one later step (alignment X2-11).

Answers to the consistency check (CC) and reviews A and B of 30 Oct, for every item that concerns
this PRD. "Accepted" means the change is in the body above.

| Item | Answer |
|---|---|
| CC 1, B verdict 1: asks not absorbed | Accepted. PRD-00 and PRD-01 revision 2 took them; the "Asks" section now gives each one's status |
| CC 7: U1 and S7 seams missing | Accepted, done in PRD-00. U1 grew three fields (Interfaces, PRD-08 row) |
| CC 9: owners of `usage-job.sh` and `usage_fixtures/` | Accepted, done in PRD-00; the job is third slice |
| CC 19: server access during the build | Accepted. Read-only database and R2 credentials on the conductor only; the job is proven there against staging and `goalcup-usage-staging` (sections 1 and 7, risks, files) |
| CC 30, 62: `set` on database copies | Accepted, done in PRD-01 and PRD-04. The reader groups by `set`, newest readable first (section 5) |
| CC 40: PRD-01 asks 3 and 4 | Ask 3 done in PRD-01 section 9. Ask 4 withdrawn: the converter is cut |
| CC 83, B M4, B A9: `rusqlite` and `zstd` | Accepted. PRD-08 declares both in phase 0 (PRD-00 ruling); `sqlite.rs` itself waits for the second slice |
| CC 84: rate-limit range | Accepted. 95 to 99 everywhere in this PRD (it already was) |
| CC 85: U1 outcome enum | Accepted. PRD-09 owns U1; PRD-08 adopts it |
| CC 102: cited history docs | Accepted. PRD-00 keeps them in this build; the PRD-15 row says so |
| CC 110: host kind source | Accepted. Kinds come from `conn` meta or the `start` event, never an embedded `model-hosts.toml`; `other` added (section 7, PRD-11 row) |
| CC 111: `LUNA` | Accepted. One line in section 8 |
| B verdict 3, B R1: Hermes cannot pass C1 | Accepted with PRD-00's default (question 19), and narrowed: Hermes's own storage doc lists `messages.timestamp` and `sessions.api_call_count` (checked on the vendor's v0.22.5 doc, unmeasured), so the per-call join is the plan and the per-session count the fallback (sections 4 to 6, proof 3) |
| B A7: unreadable snapshot | Accepted. Newest readable set used; none readable gives `readable: false`, `snapshot_unreadable`, PRD-08 skip, C1 INCONCLUSIVE (section 5). Superseded by ledger T4: C1 is judged from the record |
| B A8: native readers for other harnesses | Accepted. In 1.0 only Codex, Claude Code and Hermes; others `readable: false`, `no_reader`, C1 INCONCLUSIVE "not verified: no log reader" (section 5). Superseded by ledger T4: the local log check is `skip` and C1 is judged from the record |
| B N1: run 1 converter | Accepted (PRD-00 ruling). Section 8 is now the target shape; proof 6 is A1; `usage_run1.rs` keeps the synthetic shape |
| B N3: job, bucket, CSV, text, rollup | Partly, as PRD-00 ruled: `usage --json` in the core slice; job, bucket, `run.json`, CSV and `--text` in the third slice, not after merge, because the owner asked for stats in this build |
| B N4: live pricing | Accepted (cut by PRD-00). No `--live` mode here: an open entry is read to its last whole frame. The ledger line comes from the summary after the run |
| B T7: per-call token equality | Accepted. Proof 1 passes on per-thread sums of matched calls; per-call mismatches are reported only |
| B T9: byte-pinned summary | Accepted. `usage_out.rs` asserts facts and determinism, not bytes |
| B O5: start at H+0 | Accepted. Builder notes |
| Review A finding 3: fail-closed masking on a WebSocket direction | Accepted as it touches this PRD: `boundary()` bounds the damage to one message; the summary counts calls with `fail_closed` masks (`status.fail_closed`, risks) |
| Review A finding 9: SQLite captured only at the seal | Fixed in PRD-04 (hourly copies). This PRD reads the newest readable copy and says how far it reaches (`native_until`) |
| Claude Code side calls and `unattributed` 0 (found while checking PRD-12 C5 and PRD-16 X8 against this PRD) | New: `unattributed` now means only a hint-bearing harness's call with no hint; join-placed harnesses' leftovers are `wire_only` (section 5); asks of PRD-12, PRD-16 and PRD-08 added |
| Owner's question on gateways ("one system, not a patchwork") | Confirmed in the design: one reader; the path picks the shape, the host only labels it; a builder trap forbids per-gateway branches (section 3, section 7, builder notes) |

## Alignment answers

The alignment pass of 30 Oct (fix lists g1, g2 and g3), checked against OWNER-DECISIONS.md, PRD-00,
PRD-01, PRD-05, PRD-08, PRD-12, PRD-16 and the old code.

| Fix | What was done |
|---|---|
| L5-01 | Applied the PRD-09 part. Owner decisions: no contest rule reads the numbers (D26); the one reader that acts on them is the pad's per-run spend check (T6, PRD-16 section 9), every 5 minutes during every pad run and A1 to A3, on the open local entry in the box. Merged with G3-X11. Rates named as "the pad's rates in `scripts/pad/pad.toml` (PRD-12 section 7)", since PRD-12 and PRD-00 word them differently (flat per key, per model); the rate table is PRD-12's |
| L6-05 | Applied the PRD-09 part, merged with G3-X1. Section 1 now splits the build job: the conductor lists, fetches, copies in and puts; `goalcup-verify usage` runs in a build box with no credential (B1). Same split in `scripts/usage-job.sh` (`--box`), risks, builder note 7, the watcher note and the proofs heading. Matches PRD-00 "Where the build runs" as it now reads |
| L7-10 | Applied. The rule for adding a harness says how a pad sample becomes a fixture: an excerpt from the masked record, `goalcup-verify secrets` at 0 hits (in a build box) in `provenance.json`, named in the PR for reviewer A to read by eye |
| X2-11 | Partly. Moving the review sections into a new `REVIEW-HISTORY.md` is not done here: this pass edits only this file, and many writers of one new file would race (as PRD-00 and the other PRDs answered). "Review answers" now opens with a line saying it is history and that the body, the ledger and these answers win; the ruled asks table says the same. Every ruling still binding is in the body. The move may run once, after every alignment pass, by one agent |
| G3-X1 | Applied, merged with L6-05. Proof 6 says "in a build box" |
| G3-X7 | Applied. PRD-08 row: `native_form` (Hermes `PerCall` with usable times, else `Totals`) and per-thread `native_total` (the session's `api_call_count`). Section 6 and proof 3 use the per-entry totals comparison of PRD-08 section 3.1. Asks 3 and 4 added; PRD-08 section 3.1 already reads it this way, R5 still names `totals` only, so ask 4 says so instead of "done" |
| G3-09a | Applied. Dedupe by (`path`, `gen`, offset) before parsing; the Codex rule is now "larger than the largest total seen in that file" in the token table and the new paragraph; `usage_join.rs` case for a repeated rollout tail; the watcher's plain count follows the same rule |
| G3-09b | Applied. Section 5 paragraph on entries after `continue` (read with the run's earlier entries of the same `path`; else the Codex thread id from the file name, parent and model `null`). Section 6 uses the run's start |
| G3-09c | Applied. Path matched with the query removed; `usage_shapes.rs` case for `?beta=true` and `?alt=sse` |
| G3-09d | Applied. Claude Code falls back to the entry-wide time join; `coverage.joined_by_time` added to the summary and named in U1; proof 4 and a `usage_join.rs` case say so |
| G3-09e | Applied. Copilot CLI, Factory Droid and Cline CLI read `wire_only` until a pad sample, `readable: false`, `no_reader` |
| G3-09f | Applied. Proof 6 compares merged tokens of `matched` and `native_only` calls with the watcher's sum, `wire_only` listed apart; time logged, not a pass line. Section 8's A1 sentence changed to match |
| G3-X11 | Applied, merged with L5-01. PRD-16 row: every 5 minutes on the open entry of every pad run and of A1 to A3 (PRD-00 "Not in this build" says every pad run, not only `full` rows; that wording was used) |
| G3-X30 | Applied. PRD-16 and PRD-12 rows and new ask 5: every `ok` call has a model; tokens present or counted in `tokens.calls_without_usage`; refused calls have no usage |
| G3-09g | Applied. Hermes label is none; `sessions.title` is text from the run |
| G3-09h | Applied. Amp token row, `thread.rs` copy and `shapes::amp` removed; Amp hint row is "none until a pad sample"; the cut table says `thread.rs` is cut whole |
| G3-09i | Applied. Question numbers now read "question 1, old 4", "question 1, old 1 and 7" (line 42 named both), "question 1, old 7" and "question 7, old 19", in the body, defaults, the PRD-10 row and the ruled asks table. The review answers keep their old numbers as history |
| G3-09j | Applied. Proof 1: `native_only` 0 outside named GAPs and the seal cut |
| G3-09k | Applied. Proof 2: per-thread token equality over matched calls; per-call mismatches listed |
| G3-09l | Applied. Proof 5 names the simulated dodger of PRD-12 section 2 |
| G3-09m | Applied. Builder notes: read the vendor's current docs and cite the page before `native.rs`, `sqlite.rs` or a shape (T7) |
| G3-09n | Applied. 5,704 + 1,279 (checked with `wc -l`) |
| G3-09o | Partly. PRD-05's line numbers have moved since the review, so the row cites PRD-05 section 7, its PRD-09 interfaces row and A3 by name. The row also now says a rate-limit header whose name matches the name rule stays masked (PRD-00 owner question 11, default no), which this reader does not need |
| G3-09p | Applied. `codex.rs:163-166` (checked in the code) |
| G3-09q | Applied. `usage::Calls::new()`; the sink reads the `start` EVENT from the pushed records; no `Start` type in F1 |
| B10 sweep (30 Oct) | Applied. One test file, `usage_run1.rs` (about 200 lines), as PRD-14 section 3 says; it gains the point 4 cases (no body text in output, VR2 panic). Shapes, joins, SQLite repair and output are proved by PRD-12 C5 on real runs and the watcher's count; `usage_shapes.rs`, `usage_join.rs`, `usage_sqlite.rs` and `usage_out.rs` are cut. Builder note 1 logs the fixture numbers once instead of pinning them |

## Amendments

### 2026-10-30, replan after PRD-07 slice 3 (merge 323b1a0f)

- **`delete-test-runs.sh` now cleans the usage buckets** against S7's final shape, before the job
  exists. It empties `usage/test/` in `goalcup-usage` and `usage/test-staging/` in
  `goalcup-usage-staging`, and with `--older-than` keeps only keys of the form
  `usage/{event}/{run}/...` whose `{run}` is a deleted test run. So the job must:
  - write every object under `usage/{event}/{run}/` exactly as S7 says (`{entry}.json`,
    `{entry}.calls.jsonl`, `run.json`), with `{event}` the run's own event id (`test` or
    `test-staging` for organizer test runs, the Worker's `TEST_EVENT_ID` values) and `{run}` the run id
    as the database holds it;
  - write a production run only to `goalcup-usage` and a staging run only to `goalcup-usage-staging`.
    The script looks for `usage/test/` in the first and `usage/test-staging/` in the second only, so a
    production test run's usage in the staging bucket would never be cleaned. With
    `GOALCUP_USAGE_BUCKET` defaulting to the staging bucket in the build, the job reads staging runs
    only.
  A slice that changes the S7 layout posts a seam line and asks PRD-07 to change the script in the same
  merge.
- **A usage bucket the cleanup token cannot reach is skipped**, not an error (`403`, or `404` for a
  missing bucket). The owner's token for the script needs read and write on each usage bucket that
  exists; PRD-15 writes that into OPERATING with the usage job section.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 1 (merge d10feef7)

- **The placeholder is on integration:** `crates/goalcup-verify/src/usage/mod.rs` (12 lines),
  `pub fn run(args: Vec<std::ffi::OsString>) -> std::process::ExitCode`, `args` being those after the
  word `usage`; `lib.rs` has `pub mod usage`. PRD-09's first merge replaces the file whole (not a
  text merge) and keeps that signature. `main.rs` dispatches `usage` through clap with its help flag
  off, so `usage --help` reaches PRD-09's code.
- **Dependencies are declared once** in PRD-08's `crates/goalcup-verify/Cargo.toml`: `goalcup-format`,
  `zstd`, `serde`, `serde_json`, `sha2`, `clap`, `regex`, `base64` and `rusqlite` 0.41 with
  `bundled`. PRD-09 adds none without a seam line to PRD-08, which owns the file.
- PRD-08's next slice builds `legs.rs` and `history.rs` on U1 (stub first, judge 12:19Z); U1 stays as
  PRD-09 publishes it.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-09 slice 1 (merge f5a529c8)

- **U1 as merged, the shape later slices keep.** `crates/goalcup-verify/src/usage/mod.rs` replaces
  PRD-08's placeholder; `usage::run(args: Vec<OsString>) -> ExitCode` keeps its signature (the stub
  says the reader is not built and exits 3). `Calls::new()` takes no argument: the sink reads the
  `start` EVENT from the pushed records. `CallRow` has the section 6 fields plus `host`,
  `host_kind`, `shape`, `via`, `detail`, `effort`, `extra` (a map of unknown integer usage members)
  and `tokens: Tokens` (each count `Option`, never 0 for "not said"); `usage_from` is
  `Option<Leg>`. `CallTable.children` is `Option<map>`: `None` means parentage was not read, and
  PRD-08's `subagents_present` skips on it (it no longer follows from `Reason::NoReader`).
  `CallRow::new(leg, entry, boot, kind)`, `Thread::new(id)` and `CallTable::default()` are the only
  ways to build these; callers set fields or use struct update from them, never a full struct
  literal, so a field added later breaks no caller. Wall times are nanoseconds since the Unix epoch,
  as LINK `wall_ns`.
- **Nit B10 is taken in the core slice (replan decision).** `Thread` and `CallTable` each hold one
  fact in two fields (`readable: bool` and `reason: Option<Reason>`, "None exactly when readable is
  true"). The core slice drops both `readable` fields, keeps `reason` (`None` means readable) and
  adds `pub fn readable(&self) -> bool { self.reason.is_none() }` to each. S7's JSON still writes
  both `readable` and `reason` from that one field, so PRD-12 and the job see no change. Nothing on
  integration reads U1 yet, so this is a few lines smaller at no cost now. The slice posts it as a
  seam line to PRD-08. If PRD-08's `legs.rs` or `history.rs` reaches integration first and reads the
  field, the fields stay as merged and B10 is closed as declined: no cross-lane churn for a nit.
  `Thread::new` stays written by hand either way (a derived default would mean readable).
- **Fixtures: section 9's list is the whole copy.** `tests/usage_fixtures/` holds the Codex
  WebSocket run, the Claude Code wire excerpt and the four `m2-native` excerpts, each folder with a
  `provenance.json`, sha256 checked against the old crate. The Claude Code wire excerpt was turned
  into the Codex fixture's line format and keeps no request-id header (the source had none); the
  `m2-native` provenance comes from the old `m2_native_observations.rs` doc comment (the old folder
  had no provenance file). The `connectors` fixtures named in the "Today" fate table and in PRD-13's
  move table are not copied: they are the old decoders' expected-count snapshots (ledger B10, no
  golden snapshots of internal shapes) and only point at the `m2-native` files already copied.
  PRD-13 deletes them with the old crate.
- **Size, `wc -l` at f5a529c8:** `usage/mod.rs` 304 (89 of them doc comments) against its own
  estimate of 250, with `run`, the `Calls` sink, the join, totals and output still to come. The extra
  lines are the final U1 types the estimate did not size (the seven `CallRow` fields above, `Tokens`,
  `HostKind`, `Shape`, `Via`, `Join`, the constructors). The row (about 1,470 src) stands: the core
  slice reports each file against section 2's estimates, keeps the type docs to one line where one
  line says it, and names the point any line past 1,617 (110% of the row) serves in a decision line.
  Tests: no `usage_run1.rs` yet (the 200-line test row is untouched; the stub's checks ride on
  PRD-08's tests). Fixtures are 555 lines of data, outside both rows.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-09 slice 2 (merge dab0ce1f)

- **On integration:** `shapes.rs`, `sse.rs`, `wire.rs` and the `Calls` sink in `mod.rs`. Every wire
  call of an entry (HTTP and WebSocket, any host, shape picked by path suffix) is a `CallRow` with
  model, effort, the five token numbers, `extra`, outcome, detail, `whole`, host, `host_kind` and
  times, in (boot, sid, msg) order. `finish()` returns those rows with the rest of
  `CallTable::default()`: `reason: Some(NoReader)`, no threads, `children: None`,
  `native_form: None`. `usage::run` is still the stub (exit 3). Rules the next slices keep, as merged
  (lane log 15:08Z to 15:36Z): warm-ups and interrupts are rows of their own kind, never calls; a rate
  limit sent as both a stream `error` and a `response.failed` is one row; a WebSocket message still
  open at a new boot's PART or at the end of the records is cut, so a crash-cut create is a call with
  `whole: false`; a GAP marks a call not whole on half-open ranges, with a loss ending at a message
  start counted (A8) and a `lost 1` GAP hitting only when its offset is inside the range (B9).
- **B10 is done:** `Thread.readable` and `CallTable.readable` are gone; `readable()` methods read
  `reason`. No other lane read the fields (PRD-08 note).
- **Size decision** (review B11; `wc -l` at dab0ce1f, same as bce2f879; row 1,470 src and 200 tests).
  Src 1,525, 104% of the row with 4 of 8 files written: `wire.rs` 601 (estimate 250), `shapes.rs` 456
  (300), `mod.rs` 407 (250, of which 85 doc-comment lines), `sse.rs` 61 (40); these four are 182% of
  their 840-line estimate. Tests 299 (`usage_run1.rs`, 6 cases), 150% of the row. Held to section 2's
  estimates, the rest (`native.rs` 250, `sqlite.rs` 150, `hints.rs` 80, `out.rs` 150, the join,
  totals and `run`) puts src near 2,300, so the 110% line (1,617) cannot hold. The extra lines serve
  point 1 (every call counted once, cut and lost calls kept as not whole, the restart cut) and ledger
  S2 (per-shape model and token tables for the five numbers), not features. This replan sets the
  PRD-09 ceiling to **2,250 src and 380 tests**, with these limits for what is left: `native.rs` 250,
  `sqlite.rs` 150, `hints.rs` 80, the join, totals and `run` at most 130 more in `mod.rs`, `out.rs`
  120 for S7 (`{entry}.json`, `{entry}.calls.jsonl`, `run.json`). Cuts made first, before anything is
  added: `mod.rs` type docs to one line where one line says it (section 2 rule, about 30 lines);
  B12 below. In the third slice `usage table` (the CSV) and `--text` are built only if they fit
  under 2,250, the CSV last: no PRD reads either (the per-thread rows are in the summary JSON and
  `--text` waits on PRD-00 question 1); acceptance lists one not built as cut. Past 2,250 or 380 needs
  a new `decision` line. The end-of-build report counts this against the Rust totals.
- **Open nit B12, taken at the next touch of `usage_run1.rs`:** `a_create_cut_by_a_restart_is_a_call_not_whole`
  builds its create with a hand `format!`; it uses `create("ws-x", None).to_string()` like the other
  cases (same model name, same MARK, the 40-byte cut still lands inside the message), so the file has
  one way to build a create.
- **Declined, accepted by both reviewers:** A3 (pending creates are not dropped at a `ws_down` GAP;
  `previous_response_id` bounds the shift) and the `End::Failed` arm merge (no line saved).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-12 slice 4 (merge 69f355c6)

- The pad's spend check, as merged, runs `nice -n 19 goalcup-verify usage <entry> --json` on the open
  entry every 5 minutes and prices `models[]` `model` and `tokens` `input`, `cached`,
  `cache_write` and `output` (`reasoning` inside `output`, never added again). A rename of those
  keys is a seam line, since the T6 cap reads them; exit 3 stays "not read", never 0 USD.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-04 core slice part 3 (merge a302d688)

- Until PRD-04's `snapshot.rs` lands, a `final` row's database reaches the record only at the seal,
  as single `snapshot` streams with `form: "snapshot"` and no `set` or `harness_exited` (PRD-04
  note). The usage reader builds no reading path for that interim shape and takes no fixture from a
  record made before that slice; it reads databases by `set` as PRD-01 section 4 states.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-09 slice 3 (merge 1a2c3537)

- **On integration:** `hints.rs` (the connector's reader and thread hints) and `native.rs` (Codex
  rollouts, Claude Code transcripts and their subagent transcripts), and the join in `Calls::finish`.
  Rows now carry `thread`, `parent` and `join`; native rows are `usage_from` where the join picks the
  native leg; `CallTable.children` is `Some` once a native leg was read; matching is by id first, then
  by time (`joined_by_time` counted). Timestamps that do not parse are no time (the row keeps `None`),
  never a stop (F1); a thread counts as readable only when it owns a read session file (F2).
- **Gate:** `gate fast: PASS in 119s` on 1a2c3537 (nextest 219 of 219, 0 skipped). Box runs before the
  merge: reviewers A and B (fmt, clippy `-D warnings`, 32 and 35 tests with probes for the id join,
  the time join and the restart repeat), the fixer at 7b04a910 and 13596574 (fmt, clippy, doc
  `-D warnings`, `goalcup-verify` 32 of 32), each re-checked by a reviewer. Six paths changed, all
  PRD-09 owned, no `Cargo.toml`. `gate full` was not run.
- **Size decision** (`wc -l` at 1a2c3537; the slice 2 ceilings are 2,250 src and 380 tests). Src
  2,137: `mod.rs` 501, `wire.rs` 607, `shapes.rs` 456, `native.rs` 445 (limit 250), `hints.rs` 67,
  `sse.rs` 61. Tests: `usage_run1.rs` 381, one past 380. With `sqlite.rs` and `out.rs` still to come
  the crate projects to about 2,450 src, which is 167% of the row (1,470) and 109% of the 2,250
  ceiling. The extra lines serve point 1 (every call counted once, on either leg, joined by id and
  then by time) and point 2 (every Codex thread and every Claude Code subagent transcript becomes a
  thread with its parent), not features; the reviewers found no cut that keeps the behaviour (B-2,
  N3, B-3 and B-4 declines accepted). **Decision: the ceilings become 2,450 src and 400 tests.**
  Limits for what is left: `sqlite.rs` 150, `out.rs` 120, `run` and totals at most 40 more in
  `mod.rs`; no further line in `native.rs` or `wire.rs` unless a fix nets to zero. The CSV
  (`usage table`) and `--text` are built last and only if they fit under 2,450; acceptance lists one
  not built as cut. The 20 extra test lines are for one `sqlite.rs` case (a database copy read by
  `set`, and the unreadable copy giving `SnapshotUnreadable`); no other test is added (ledger B10).
- **Open nits, taken at the next touch of the file, all same-size:**
  - A2-1 (`native.rs`, the `token_count` repeat guard): `total_tokens` that is present but not an
    unsigned integer (null, float, negative) reads as `None`, and `None <= top` skips the call as a
    repeat. Use `let Some(total) = ...as_u64() else { return None };` before the comparison, so only a
    real repeat reaches `repeats_skipped`. Same shape as the B-1 fix.
  - A2-2 (`mod.rs`): a thread that only the wire names is pushed with the table-level `reason`, so
    it is `readable()` with `native_calls` 0 beside a spawn-only thread that is not. Keep the
    behaviour and say so in the `NativeLeg::threads` doc: a wire-only thread has no read file and
    its readability is the table's; consumers read `native_calls`, not `readable()`, for "this
    thread has native rows" (PRD-08 note). The parent backfill (N4) runs before native rows are
    appended, so a native row of a Codex file without `session_meta` whose thread only a parent's
    spawn names keeps `parent` `None` while its thread entry has the parent; move the backfill after
    the append if it costs no line, else leave it and let `parents` in the thread list be the source.
  - A2-3 (`usage_run1.rs`): the Claude Code case stamps every line 2300-01-01, so no Claude native
    row has a time and nothing in the file covers the Claude time join. Stamp one line 2300 and the
    others with a normal time when the file is next touched (no extra line); the rig's `cycle` runs
    cover the real Claude time join (ledger B10).
- **For the next slice:** `sqlite.rs` reads a database by its `set` (PRD-01 section 4) and builds
  no reading path for the interim single-`snapshot` shape (earlier note, PRD-04 part 3); `out.rs`
  writes S7 from `Thread`/`CallTable` as merged, and the pad's spend keys stay as the PRD-12 note
  names them.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after the PRD-09 slice 4 review (`out.rs` and `usage`)

- **Fixed in this slice:** `calls.jsonl` no longer carries `path` (a session path can name the
  participant's folders; section 7 keeps paths out of both files, and the test now plants MARK in a
  Codex session path to guard it). `detail` is the row's own `detail` once, not status and code twice.
  A decode error partway through an entry gives the summary with `read_error` true and a line on
  stderr, never a clean-looking partial count; a torn tail is still not an error. A run folder
  goes on past an entry it cannot read, and exit 3 means no entry gave output. Output files are named
  `<name>.json` and `<name>.calls.jsonl` whole, so a folder name with a dot is not cut. Thread rows
  carry `models` (model to call count) and `effort`, which the third-slice CSV and the PRD-16
  hand-off table read. `coverage` shows `repeats_skipped` and `copied_skipped`. `harness_version`,
  `connector` and `run` get the same printable 128-byte bound as other labels.
- **Cut from the S7 summary, as a decision:** `coverage.by_10min`, `status.fail_closed` and
  `threads[].source`. Nothing reads them today (the pad's spend check reads `models` and `tokens`);
  a lane that needs one adds it with its reader.
- **Seam line to PRD-08:** `CallTable` has five more public U1 fields (`run`, `entry`, `connector`,
  `harness_version`, `sealed`), filled from the `start` event and the last PART; the table built
  from `CallTable::default()` is unchanged for existing callers.
- **Covered by inspection, not by a test:** "a panic inside `usage` prints no record text". The
  hook is PRD-08's (`main.rs` prints only file and line and turns the panic into exit 3); no test in
  `crates/goalcup-verify/tests` covers it, and this PRD adds none (ledger B10).
- **Size decision:** the slice 4 limits (`out.rs` 120, `mod.rs` at most 40 more, tests 400) were
  missed: `out.rs` carries the per-thread rows (models, effort) that the schema and ledger S2 ask
  for, `mod.rs` the entry reader that must go on past a bad entry and mark a stopped read, and
  `usage_run1.rs` one more point-3 case. **Decision: the ceilings become 2,550 src and 500 tests** (`wc -l` at this slice: src 2,392 before `sqlite.rs`, tests 500);
  `sqlite.rs` stays at 150 and the CSV and `--text` stay last and only if they fit.

### 2026-10-31, replan after the PRD-09 slice 4 re-review (A2-1 to A2-3, N1)

- **Fixed:** `harness_version` keeps its spaces (`codex-cli 0.50.0`, `2.0.14 (Claude Code)` reach the
  summary): it is checked only for printable ASCII, spaces allowed, at most 128 bytes, the rule the
  CLI's `reported_line` uses; `run` and `connector` still go through `hints::id`. The output file
  name is the entry id only when it is non-empty and all ASCII alphanumeric, else the folder's
  name, else `entry` (a path with no folder name, as in `usage . --out o`), so no entry id can
  write outside `--out`.
- **Seam line to PRD-12 (S7 key `read_error`):** S7's summary has a top-level boolean `read_error`.
  `true` means the read of that entry stopped at a decode error and every count is partial (usage
  still exits 0). The spend check (ledger T6) must treat a summary with `read_error` true as not
  read or as a lower bound, never price it as whole; the pad checks C1, C2 and C5 must report
  INCONCLUSIVE for it, not judge partial thread and token counts as the whole run. The files that
  do this (`scripts/pad/pad-run`, `scripts/pad/check.py`) are PRD-12's; PRD-09 does not edit them.
  S7 keys therefore include `read_error` alongside the three widened keys. Until PRD-12 reads it,
  a partial read is visible in the stderr line and in the key, and nowhere else.
- **Size decision, stated plainly:** the PRD-00 Size budget row for the usage reader is 1,470 src
  and 200 tests. The slice is at 2,396 src (`wc -l`: hints 67, mod 612, native 447, out 146, shapes
  456, sse 61, wire 607), about 63% over, and 500 tests in `usage_run1.rs`, 2.5 times over. The
  causes are the ones given in the earlier size decisions (both legs of a call joined by id, the
  entry reader that goes on past a bad entry, thread rows, many harness shapes); no cut was found
  that keeps behaviour. The ceilings stay 2,550 src; `sqlite.rs` (150) takes the rest. The tests
  ceiling becomes **520** (`usage_run1.rs` is 502 after rustfmt): the rest of the allowance is for the one `sqlite.rs`
  case (a database copy read by its `set`), and the case goes in `usage_run1.rs`.
