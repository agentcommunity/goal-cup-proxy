# Live stats during the week (design)

Design, 22 Oct 2026. Not built. What a public page can show while the agents run (7 to 15 Nov), where
each number comes from, and the smallest way to build it. No code in this step.

The owner's ask: show during the week how many tokens were used, how many PRs, how many agents, maybe
how many lines of code, how often the agents failed, and other things players find interesting. Keep it
simple.

## The short version

- **Three sources, built in this order.** (a) The database the upload server already fills: agents
  running, harness mix, hours of work, model calls, data recorded, quiet and idle notices, restarts.
  Buildable before 7 Nov. (b) A verify job that reads the stored traces with `goalcup-verify` and
  sends back one small numbers-only summary per run: tokens, models, tool calls, tool errors,
  subagents, "keep going". (c) A GitHub reader for the public repositories: commits, pushes, PRs,
  lines changed.
- **One public file**, `GET /v1/live.json` on the upload Worker. The Worker's 5-minute cron writes it
  to R2; the GET serves that copy. The website page `/goalcup/live` renders it.
- **Community totals only.** No per-entry numbers, no ranks, no "top entry". A number is shown only
  when at least 5 runs feed it. Test and practice runs are never counted.
- **Nothing here is judged.** Tokens, lines and calls say how much work happened, not how good it is.
  The page says so.
- **"Failed" is never counted by searching text.** A tool error is a tool result the harness itself
  marked as an error, counted once per tool call. See [Failures](#failures-and-why-not-grep-failed).

## What is stored today

All in schema `goal_cup_tracer` (the mirror is
[`server/worker/sql/goal-cup-tracer-upload.sql`](../../server/worker/sql/goal-cup-tracer-upload.sql),
plus [`sql/pending`](../../server/worker/sql/pending/README.md) for `stored_bytes`, vitals and the
disk pause). The website owns the schema; `signups` lives in the website's migrations.

| Table | What it gives a stats query |
|---|---|
| `events` | `mode` (`competition`, `dry_run`, `practice`, `test`), `staging`, start window, `run_hours` |
| `runs` | `mode`, `official_start`, `deadline_at`, `replaced_at`, `status`, `last_heartbeat_at`, `last_model_request_at`, `flag_reason`, `disk_paused_at`, `repo_url`, `final_*` |
| `entries` | one per segment: `segment`, `connector_id`, `harness_version`, `model_requests` (highest cumulative count the entry reported), `stored_bytes`, `chunk_count` |
| `heartbeats` | every 5 minutes per entry: `received_at`, `model_requests` (cumulative per entry), `recording` |
| `notices` | `kind` (`started`, `silence`, `idle_3h`, `idle_6h`, `recording`, `ended`, `disk_full`) per run |
| `signups` (website) | active sign-ups per event (`withdrawn_at IS NULL`), `anonymous`, `research_opt_in` |
| R2 `cron/health.json` | the cron's last tick and since when the server has been well ([UPLOAD-API](../UPLOAD-API.md#what-the-server-does-on-its-own)) |

Tokens, models, tool calls and human messages are not in the database. They exist only inside the
raw traces in R2, and only the verifier's decoders can read them.

**Which runs count.** A public number counts only runs with `mode = 'competition'` in the current
event (`EVENT_ID`: the dry-run event until the 6 Nov switch, then the cup) on a non-staging event.
That leaves out organizer test runs (`mode = 'test'`, their own event; the test-mode migration says
"aggregates must leave out runs with mode = 'test'", and [SERVER-DESIGN](../SERVER-DESIGN.md#test-runs)
says the same) and practice runs (`mode = 'practice'`, their own event, 10 minutes, repeated at will).
"Work" totals (hours, calls, tokens) count every competition run of the event, replaced ones too,
since that work happened. "Entry" counts (started, running, finished) count main runs only: a run a
later start replaced is not an entry.

## The stats

Source: **DB** = the database now; **verify** = the verify job over stored traces; **GitHub** = the
GitHub reader. Privacy: **aggregate** = community total only, shown when at least 5 runs feed it.
Before 7 Nov: whether it can be built, tested and deployed before the start window opens.

### From the database (step a)

| Stat | How it is computed | Cadence | Cost | Before 7 Nov |
|---|---|---|---|---|
| Entries signed up | active `signups` of the event | 5 min | one count | yes, once the sign-up migrations are applied; `null` until then |
| Agents started | main competition runs of the event | 5 min | one count | yes |
| Agents running now | main runs not past their end whose `last_heartbeat_at` is under 20 minutes old (the `silence` threshold) | 5 min | one count | yes |
| Agents working now | running, and `last_model_request_at` under 30 minutes old | 5 min | one count | yes |
| Agents quiet now | main runs not past their end with no heartbeat for 20 minutes or more | 5 min | one count | yes |
| Finished | main runs past their end; split into `sealed` and `incomplete` once closed | 5 min | one count | yes |
| Harness mix | `entries.connector_id` of segment 0, per main run; a harness with fewer than 5 runs folds into `other` | 5 min | one group-by | yes |
| Agent-hours recorded | per run, the number of distinct 5-minute slots with at least one heartbeat, times 5 minutes; summed | 5 min | one pass over `heartbeats` of the event (about 600k rows at the end of the week; the `(run_id, received_at)` index exists) | yes |
| Agent-hours working | the same, counting only slots in which the run's model calls grew | 5 min | same pass | yes |
| Model calls | sum over entries of `entries.model_requests` (each entry counts from its own start, so a `continue` adds, never double counts) | 5 min | one sum | yes |
| Model calls per hour | per entry, positive differences between consecutive heartbeats' `model_requests`, summed per UTC hour; plus distinct runs beating per hour | 5 min | window function over the same pass | yes |
| Restarts | entries with `segment > 0`: how often a participant ran `goalcup continue` after the agent died | 5 min | one count | yes |
| Data recorded | sum of `entries.stored_bytes` (compressed bytes in R2) | 5 min | one sum | yes |
| Quiet, idle and disk notices | count of `notices` rows per kind (`silence`, `idle_3h`, `idle_6h`, `disk_full`) | 5 min | one group-by | yes |
| Still going after 24, 72, 120 and 168 hours | of runs that started at least that long ago, the share whose last heartbeat came at or after that hour | 5 min | one pass over `runs` | yes (fun, cheap) |
| Busiest hour | the largest point of "model calls per hour" | page | none | yes (fun, cheap) |
| Server uptime | share of 5-minute ticks since the first official start in which the cron ran and `cron/health.json` said the server was well; outage windows as a list | 5 min | carried forward in the snapshot | yes |

Not shown: IP addresses, countries, machine facts, vitals, workspaces, session ids, repository URLs
of anyone, run ids or entry numbers. `runs.flag_reason` (recording problems) and the number of agents
paused for disk right now are aggregate and harmless, but they describe the tool more than the race:
off by default, see the owner questions.

### From the traces (step b, the verify job)

Every number here comes from the per-run `--stats` summary (schema below), which the verifier
computes with the de-dup rules of [CONTRACTS C4](../CONTRACTS.md#c4-status-and-interpretation) and
the [report format](m2-report-format.md#reading-it-honestly): never sum cumulative usage, retries or
replayed history into unique totals, and never add the native and the wire count of the same call.

| Stat | How it is computed | Cadence | Cost | Before 7 Nov |
|---|---|---|---|---|
| Tokens in, out, cache read, cache write | per run from one source, see [Tokens](#tokens); summed over runs that have a summary; shown with "from N of M runs" | every 6 hours | download and decode every run (see [Cost](#cost)) | code and tests yes; the job's deploy needs the owner |
| Models used (top 8) | `model` observations of the source the run's tokens came from, counted per response; a model shows only when at least 5 runs used it | every 6 hours | same | yes, same caveat |
| Tool calls | distinct supplied tool call ids over every decoder of the run (native and wire share ids such as `toolu_…` and `call_…`); calls with no id are counted from the token source only | every 6 hours | same | yes |
| Tool calls that came back with an error | distinct tool call ids with any result marked `is_error: true` (Claude Code, Anthropic wire) or a shell `exit_code` other than 0 (Amp); "from N runs that mark errors" | every 6 hours | same | yes |
| Subagents spawned | distinct ids of tool calls that start a subagent: `Task` and `Agent` (Claude Code, Amp), `spawn_agent` (Codex, a `CollabAgentToolCall`) | every 6 hours | same | yes; only Codex is measured on real children (two, 22 Oct) |
| Provider errors | `error` observations (safe `code` or `type` tokens such as `rate_limit_error`, `overloaded_error`), one per response | every 6 hours | same | yes |
| "Keep going" messages | `human_message` observations after the first (the prompt), one per supplied id, whose text is exactly "keep going" after trimming, lower-casing and dropping end punctuation | every 6 hours | same | yes; Claude Code and Codex only (see below) |

**"Keep going" is decodable for two harnesses.** Only native decoders emit `human_message`, and only
for a record that carries the harness's own human marker: Claude Code (`origin.kind == "human"` or
`promptSource == "typed"`; sidechains, meta and compact summaries are excluded) and Codex (`cli` or
`vscode` source, positive `user.text` marker; child and fork streams never become human input). Amp
and Hermes have no native human marker decoded today, and the wire leg cannot tell: every request
replays the whole history, and user-role text in a request is context, not a typed message. So the
stat says "from N Claude Code and Codex runs". Any other human message is compliance evidence: the
summary counts it (`other`) for the organizers, and it is never public.

### From GitHub (step c)

Every entry's repository is public by rule (`goalcup prep` checks it, the server checks it at open),
and new and empty at the start, so everything in it came from the run.

| Stat | How it is computed | Cadence | Cost | Before 7 Nov |
|---|---|---|---|---|
| Commits | default branch commit count: `GET /repos/{o}/{r}/commits?per_page=1`, read the `last` page number from the `Link` header | about hourly per repo | 1 call | yes |
| Pushes | `push` and `force_push` items of `GET /repos/{o}/{r}/activity` (the API the judged-commit read already parses in `src/github.ts`) with a timestamp at or before the run's end; at most 5 pages | about hourly | 1 to 5 calls | yes |
| PRs opened | `GET /repos/{o}/{r}/pulls?state=all&per_page=100`, counting only PRs whose author is the repository owner (anyone can open a PR on a public repo, so outsiders' PRs never count); at most 3 pages | about hourly | 1 to 3 calls | yes |
| Lines added and removed | `GET /repos/{o}/{r}/stats/code_frequency` (weekly additions and deletions, all commits); `202` means GitHub is still computing, try next round | about hourly | 1 call | yes, with the caveat below |

Lines count everything committed: lockfiles, generated code, vendored packages, build output. A
lines total is a fun fact, not a measure of work. It is shown as "lines changed in public repos,
including generated files", or not at all (owner question). After a run's end plus one hour the
reader takes one last reading and stops; pushes after the end are left out by their GitHub timestamp,
as for the judged commit. A repository that turns private or disappears keeps its last numbers.

## Failures, and why not grep "failed"

Searching the traces for the word "failed" gives a number, but not a number of failures:

- **It counts words, not events.** "0 failed" in a passing test run, an agent writing "I fixed the
  failed build", a file the agent read, a commit message, a log line: all match.
- **History is replayed.** Every model request carries the whole conversation again, so one tool
  output with "failed" in it appears in hundreds of requests. The native transcript and the wire
  carry it twice more.
- **It would need the content.** A public number would come from reading prompts, code and tool
  output, which is exactly what stays private.
- **It is easy to move.** An agent that prints less, or a prompt that says "never print failed",
  changes it without changing what happened.

What to count instead, each with a plain name on the page:

| Shown as | Definition | Source |
|---|---|---|
| Tool calls that came back with an error | A tool result the harness marked as an error (`is_error: true`, or a shell exit code other than 0), once per tool call id. This includes healthy errors: a failing test the agent then fixes, `grep` with no match (exit 1), a missing file. It is "the agent hit an error and carried on", not "the agent failed". Shown as a count and a share of tool calls. Codex and Hermes results carry no error flag the decoders read today, so they are left out and the coverage is shown. | verify |
| Provider errors | The model provider answered with an error (rate limit, overload, server error): `error` observations with their safe code. | verify |
| Agent restarts | The agent process died and the participant ran `goalcup continue` (`entries.segment > 0`). | DB |
| Runs that went quiet | `silence` notices: no heartbeat for 20 minutes while the server was well. | DB |
| Idle agents | `idle_3h` and `idle_6h` notices: still running, no model call for 3 or 6 hours. | DB |

Later, if wanted: Codex `exec_command_end` events carry an exit code; a small `codex-native` decoder
change could add `exit_code` to its tool results. That is a decoder version bump with its own fixture,
not part of this plan.

## Tokens

Each decoder reports usage in its provider's own shape, and several report the same call more than
once. The `--stats` summary turns them into four numbers per run: `input` (input tokens not served
from cache), `cache_read`, `cache_write` and `output` (with `reasoning` as a part of `output` when
the provider says). The rules, per decoder:

| Decoder | One count per | Rule |
|---|---|---|
| `anthropic-messages` (wire) | supplied message id | `input_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens` from the `message_start` snapshot; `output_tokens` from the last `message_delta` snapshot of that id (it is cumulative), else from `message_start` |
| `openai-responses` (wire) | supplied response id | the terminal event's usage; `input` = `input_tokens` minus `input_tokens_details.cached_tokens`; `cache_read` = `cached_tokens`; `reasoning` = `output_tokens_details.reasoning_tokens` |
| `openai-chat-completions` (wire) | supplied response id | the last usage chunk of the response; `input` = `prompt_tokens` minus cached (`prompt_tokens_details.cached_tokens`, or DeepSeek's `prompt_cache_hit_tokens`) |
| `amp-thread-jsonrpc` | supplied `messageId` | the last snapshot of that message: `inputTokens`, `outputTokens`, `cacheCreationInputTokens`, `cacheReadInputTokens` |
| `claude-code-native` | supplied `message.id` | one per id (Claude Code writes one line per content block with the same usage repeated) |
| `codex-native` | native session (`native_context.session_id`) | the last `cumulative` `total_token_usage` of the session; per-call `last_token_usage` is never summed (token-count events repeat). Whether a child session's total includes its parent's context is unmeasured: children are kept separate and flagged |

**One source per run, never both.** The native transcript and the wire traffic describe the same model
calls. The run's tokens come from the wire decoders when they produced usage and none of them is
degraded (the wire is what the provider was sent, subagents and side calls included), else from the
native decoders, else none. The summary keeps the other source's numbers beside it for the organizers
(`other_source`); the public total uses only `source`.

**Shown honestly:** "at least N tokens, from M of K runs". A provider that sends no usage in its
stream (a chat-completions client without `include_usage`) is a response without usage, counted in
`responses_without_usage`, never guessed.

## Architecture, smallest first

```
participant machines ── heartbeats, chunks ──► Worker goalcup-ingest ──► Supabase (goal_cup_tracer)
                                                   │  cron every 5 min:                ▲
                                                   │   notices (as today), then         │ run_stats
                                                   │   (c) GitHub reader, N repos       │
                                                   │   (a) snapshot ──► R2 live/{event}.json
                                                   │
 GET /v1/live.json (public, cached 60 s) ◄─────────┘
        ▲                                      (b) verify job (VPS or organizer machine)
        │                                          GET  /v1/stats/runs     (STATS_KEY)
 website /goalcup/live (revalidate 60 s)           read R2 traces/{event}/... (read-only token)
                                                   goalcup-verify --stats ...
                                                   PUT  /v1/stats/runs/{run_id}/trace (STATS_KEY)
```

### (a) `GET /v1/live.json` from the database

- The cron, after the notices step and in its own `try`, calls one SQL function,
  `goal_cup_upload_live_snapshot(p_event_id, p_at)`, which returns the whole document except the
  `server` block. The cron adds `server` (from `cron/health.json` and the previous snapshot), then
  writes `live/{event_id}.json` to R2, and the first snapshot after 00:00 UTC also as
  `live/{event_id}/daily/{YYYY-MM-DD}.json` for the weekly recap. A failure logs `live-snapshot-failed`
  and changes nothing else: notices never wait on it.
- `GET /v1/live.json` reads `live/{EVENT_ID}.json` from R2 and answers it. No database call on a
  request.
- Why a snapshot and not a query per request: the Cache API is per data center, so a popular page
  would query the database from many places each minute. A snapshot costs one query per 5 minutes
  whatever the traffic, keeps answering while the database has a bad minute, and its daily copies
  are the recap. Heartbeats come every 5 minutes, so a fresher number would not be more true.
- Test and practice runs are left out inside the SQL (`runs.mode = 'competition'`, the event's
  `staging` equal to the deployment's), so no caller can forget.

### (b) The verify job

This is also the verify job [SERVER-DESIGN](../SERVER-DESIGN.md#not-built) lists as not built; stats
are its first output. It runs every 6 hours during the week and once after every run has closed.

1. `GET /v1/stats/runs` lists the event's competition runs and their entries.
2. For each run whose stored bytes grew since its last summary, download the new chunks of each
   entry from R2 (`traces/{event}/{run_id}/{entry_id}/{offset:016}-{sha256}.zst`; chunks never change,
   so a local compressed copy is kept and only new keys are fetched), check each SHA-256, and
   decompress them in offset order into one `chain.jsonl` per entry in a fresh temporary directory
   (rotated chain files are one chain joined in order, and the server's offsets are over that joined
   chain). Seal and baseline documents go beside it when present.
3. Run `goalcup-verify --stats ENTRY0 ENTRY1 … --output run.json` over the run's entries in segment
   order, so ids repeated across a `continue` count once.
4. `PUT /v1/stats/runs/{run_id}/trace` with the file. Delete the temporary chain.

The job never writes a research report: `--research-json` output holds goal excerpts and text, and
`--stats` holds numbers only. The job is a small Python script in this repository
(`scripts/stats-job.py`, stdlib plus `zstd` and the S3 API), so its tests run here with a fake bucket
directory.

### `goalcup-verify --stats` (proposed)

`goalcup-verify --stats TRACE [TRACE…] --output STATS.json`. A third mode beside
`--capture-integrity` and `--research-json`, with the same output-file rules (a new file, `0600`,
outside every trace directory) and exit codes (0 complete and valid, 1 incomplete or degraded,
2 invalid, 3 no usable file). It reads each trace with `scan_capture_trace` and feeds a `DecodeSession`
as the research report does, but its sink keeps only counters and bounded id sets; no observation is
written. Schema 1:

```json
{
  "type": "run_stats",
  "schema_version": 1,
  "verifier_version": "0.7.0",
  "traces": [
    {"entry_id": "e0123456789abcdef", "exit_code": 1, "integrity": "incomplete", "sealed": false,
     "validated_records": 812345, "validated_bytes": 4123456789}
  ],
  "analysis": {"state": "partial", "degraded": ["openai-chat-completions"]},
  "tokens": {
    "source": "wire",
    "responses": 4210, "responses_without_usage": 3,
    "input": 1234567, "cache_read": 98765432, "cache_write": 2345678, "output": 3456789,
    "reasoning": 120000,
    "children_flagged": 0,
    "other_source": {"source": "native", "responses": 4100,
                     "input": 1200000, "cache_read": 97000000, "cache_write": 2300000, "output": 3400000,
                     "reasoning": null}
  },
  "models": [{"model": "claude-opus-5-5", "responses": 4000}, {"model": "claude-haiku-4-5", "responses": 210}],
  "models_omitted": 0,
  "tools": {"calls": 5210, "calls_without_id": 0, "results": 5190, "errors": 312, "error_marking": "is_error"},
  "subagents": {"spawned": 14},
  "provider_errors": {"total": 12, "by_code": {"rate_limit_error": 9, "overloaded_error": 3}},
  "human_messages": {"source": "native", "total": 9, "keep_going": 7, "other": 1},
  "bounds": {"ids_over_budget": false}
}
```

| Field | Type and meaning |
|---|---|
| `traces[]` | one per input trace, in input order: `entry_id` (from the reader facts), its own `exit_code`, `integrity` (`valid`, `incomplete`, `invalid`), `sealed`, `validated_records`, `validated_bytes` |
| `analysis.state` | `available`, `partial` or `unavailable`, as in the research report; `degraded` lists decoder ids |
| `tokens.source` | `wire`, `native` or `none` ([Tokens](#tokens)); every count below is a non-negative integer, `reasoning` may be `null` when the source never states it |
| `tokens.children_flagged` | Codex child sessions counted separately (their inherited context is unmeasured) |
| `tokens.other_source` | the same counts from the other source, or `null`; organizers only |
| `models` | at most 16, by `responses` then name; names are kept only when 1 to 64 bytes of printable ASCII, otherwise counted in `models_omitted` |
| `tools.error_marking` | `is_error`, `exit_code`, `both` or `none`: whether this run's sources mark errors at all |
| `provider_errors.by_code` | at most 16 safe code tokens; the rest under `"other"` |
| `human_messages.source` | `native` or `none`; `total` counts distinct ids, the first is the prompt, `keep_going` and `other` split the rest |
| `bounds.ids_over_budget` | `true` when an id set passed its bound (4 million ids per run); counting then continues without de-dup for that set |

Nothing in it is text from the run: no prompt, message, tool name other than the counted subagent
tools, path, URL or id other than `entry_id`.

### (c) The GitHub reader

The Worker already has `GITHUB_TOKEN`, every run's `repo_url`, the activity API parser and the
headers GitHub wants, so the reader is one more cron step, not a new service: each tick it takes the
20 competition runs whose `github_due_at` is oldest (SQL `goal_cup_upload_github_due`), makes 4 to 10
GitHub calls for each, stores the numbers (`goal_cup_upload_record_github`), and sets the next due
time an hour later. With 300 runs that is a full round about every 75 minutes and about 1,200 calls
an hour, within the token's 5,000. A run past its end plus one hour gets one last reading and is
never read again. A `403` or `429` from GitHub ends the step for that tick. The alternative is a
collector on the VPS like the events feed (`coworker_agent` → signed POST → R2); it needs a repo list
endpoint, a second secret and a deploy, so it is the fallback, not the plan.

### (d) The website page `/goalcup/live`

A server component in `agentcommunity_page` (branch `goalcup/ui`) fetches `live.json` with
`next: { revalidate: 60 }`, keeps the last good copy when the fetch fails, and renders: the numbers
as community totals, model calls per hour as a strip over the 168 hours, the harness mix, and the
trace and GitHub blocks with their "as of" time and "from N of M runs". It follows the site's design
rules and the Goal Cup mockup's look (`docs/design/goal-cup/live.js`), but not its per-entry rows:
the mockup draws one named row per entry with its model, which the privacy rules below do not allow
during the week. The weekly recap is a post written from the daily copies in R2 after the event.

## Schema delta

One new table and five functions, written here as a pending migration
(`sql/pending/…_goal_cup_tracer_live_stats.sql`) and copied unchanged to the website repo, which
owns migrations, as `sql/pending/README.md` describes. Additive only.

```sql
CREATE TABLE goal_cup_tracer.run_stats (
  run_id        text PRIMARY KEY REFERENCES goal_cup_tracer.runs (id) ON DELETE CASCADE,
  trace         jsonb CHECK (trace IS NULL OR jsonb_typeof(trace) = 'object'),   -- the --stats document
  trace_at      timestamptz,
  github        jsonb CHECK (github IS NULL OR jsonb_typeof(github) = 'object'),
  github_at     timestamptz,
  github_due_at timestamptz
);
ALTER TABLE goal_cup_tracer.run_stats ENABLE ROW LEVEL SECURITY;
REVOKE ALL ON goal_cup_tracer.run_stats FROM PUBLIC, anon, authenticated;
```

| Function | Returns | Called by |
|---|---|---|
| `goal_cup_upload_live_snapshot(p_event_id text, p_at text)` | the `live.json` document without `server` | cron |
| `goal_cup_upload_stats_runs(p_event_id text)` | `{"runs":[{"run_id","status","official_start","ends_at","stored_bytes","trace_at","entries":[{"entry_id","segment","connector_id","offset","sealed"}]}]}`, competition runs only | `GET /v1/stats/runs` |
| `goal_cup_upload_record_trace_stats(p_run_id text, p_stats jsonb, p_at text)` | `ok`, `not_found` (no such competition run) or `stale` (some entry's `validated_records` is below what is stored) | `PUT …/trace` |
| `goal_cup_upload_github_due(p_event_id text, p_at text, p_limit integer)` | `[{"run_id","repo_url","ends_at","official_start"}]`, oldest due first | cron |
| `goal_cup_upload_record_github(p_run_id text, p_github jsonb, p_at text, p_next_at text)` | `boolean` | cron |

All `SECURITY DEFINER` with a fixed `search_path`, executable by the ingest roles and `service_role`
only, like every `goal_cup_upload_*` function. `delete_test_runs` needs no change (the foreign key
cascades, and test runs never get a row). An erased member's run keeps its numbers row, which holds
no personal data.

The stored `github` object: `{"state": "ok" | "computing" | "gone" | "error", "commits", "pushes",
"force_pushes", "prs_opened", "lines_added", "lines_removed", "final": true | false}`.

## Endpoints

| Call | Auth | Answer |
|---|---|---|
| `GET /v1/live.json` | none | `200` the snapshot, `content-type: application/json`, `cache-control: public, max-age=60`, `access-control-allow-origin: *`; `404 not_ready` before the first snapshot. Like every call, `503 maintenance` while the switch is on. |
| `GET /v1/stats/runs` | `Authorization: Bearer <STATS_KEY>` (a new Worker secret; compared in constant time) | `200` as `goal_cup_upload_stats_runs`. No key configured: `500 misconfigured`. Wrong key: `401`. |
| `PUT /v1/stats/runs/{run_id}/trace` | same | body: the `--stats` document, at most 64 KiB, `schema_version` 1, checked field by field (unknown keys refused). `204` stored; `400 malformed`; `404` not a competition run of the event; `409 stale`. |

`live.json`, schema 1, with example numbers:

```json
{
  "schema_version": 1,
  "event": {"id": "goalcup-cup-2026", "kind": "competition",
            "start_window_opens": "2026-11-07T00:00:00Z", "start_window_closes": "2026-11-08T00:00:00Z",
            "run_hours": 168},
  "generated_at": "2026-11-09T14:05:00Z",
  "min_group": 5,
  "entries": {"signed_up": 412, "started": 298, "running": 251, "working": 233, "quiet": 19,
              "finished": 28, "sealed": 25, "incomplete": 3},
  "harnesses": [{"id": "claude-code-cli", "started": 131, "running": 117},
                {"id": "codex-cli", "started": 104, "running": 90},
                {"id": "hermes-cli", "started": 41, "running": 33},
                {"id": "other", "started": 22, "running": 11}],
  "work": {"agent_hours": 15873.5, "working_hours": 12011.0, "model_calls": 3184220,
           "restarts": 64, "data_bytes": 871234567890},
  "notices": {"silence": 57, "idle_3h": 140, "idle_6h": 61, "disk_full": 4},
  "survival": [{"after_hours": 24, "of": 298, "still_going": 271},
               {"after_hours": 72, "of": 0, "still_going": null}],
  "hourly": [{"hour": "2026-11-07T00:00:00Z", "model_calls": 4120, "runs_beating": 37},
             {"hour": "2026-11-07T01:00:00Z", "model_calls": 9875, "runs_beating": 66}],
  "trace": {"as_of": "2026-11-09T12:00:00Z", "runs": 240, "of": 298,
            "tokens": {"input": 1834000000, "cache_read": 41200000000, "cache_write": 2900000000,
                       "output": 512000000, "responses_without_usage": 1290},
            "models": [{"model": "claude-opus-5-5", "runs": 96}, {"model": "gpt-5.5", "runs": 88}],
            "tools": {"calls": 2412000, "errors": 181000, "error_runs": 150},
            "subagents": 3110, "provider_errors": 9120,
            "keep_going": {"messages": 233, "runs": 160}},
  "github": {"as_of": "2026-11-09T13:58:00Z", "repos": 281, "commits": 41200, "pushes": 9870,
             "prs_opened": 512, "lines_added": 18400000, "lines_removed": 6100000},
  "server": {"uptime_ticks": 615, "well_ticks": 612, "outages": [{"from": "2026-11-08T03:10:00Z", "to": "2026-11-08T03:25:00Z"}]}
}
```

Rules the document keeps: every count over runs is `null` when fewer than `min_group` runs feed it;
a harness or model under `min_group` folds into `other` (models are dropped instead); `trace`,
`github` and `models` are `null` until their source has run; `models` is `null` until the start window
closes (owner question 4); `hourly` holds at most `run_hours + 24` points from the window's opening;
`outages` at most 20.

## Fair play

Numbers shape behaviour. A board that ranks tokens or lines would reward burning tokens and
committing generated files, which is the opposite of the Cup, where judges score the final product
([RULES](../RULES.md#compliance-and-judging)). So:

- **Community totals only.** No per-entry tokens, calls, lines, errors or "keep going" during the
  week, no ranks, no "most active", no "longest running" (a top-one is a rank of one).
- **Say what is judged.** The page states that none of these numbers is judged or scored.
- **The 5-run floor** keeps a known entry (a team that announced its harness, say) from being picked
  out of a small group, and stops a single participant from putting a crafted "model name" on the
  public page.
- **Model mix waits for the start window to close.** Prompts stay private during the window so nobody
  copies them ([RULES](../RULES.md#one-prompt-then-only-keep-going)); the same reasoning applies,
  more weakly, to which models the early starters chose.
- **Nothing that helps a cheat.** No live per-run state, no idle or silence timing per run, nothing
  about verification results. Outages are shown as windows after they end; a participant whose
  heartbeats fail already knows the server is unreachable.
- **"Keep going" is aggregate and only the exact phrase.** Other human messages are compliance
  evidence for the review, and are never shown.

## Cost

- **Worker:** one SQL call and two R2 writes per 5 minutes; the GET is served from R2 behind the
  edge cache. GitHub: about 1,200 calls an hour at 300 runs, on the existing token.
- **Database:** the snapshot reads the event's heartbeats once per 5 minutes (about 600k rows by the
  end), plus small counts.
- **Verify job:** R2 egress is free; reads are one GET per chunk (a chunk every 30 minutes per entry:
  about 100k GETs for 300 runs over the week, cents). Disk: a compressed copy of every run, which is
  `work.data_bytes`, plus one run decompressed at a time. CPU: one decoder pass per changed run every
  6 hours. The per-run cap is 64 GiB compressed, so the VPS disk is the limit to check first.

## Build plan

| Step | PR | Tests | Cloud agent now (no secrets) | Needs the owner |
|---|---|---|---|---|
| 1 | This design | `scripts/check-links.py` | done | read, answer the questions |
| 2 | (a) `goal_cup_upload_live_snapshot` in a pending migration; `Db.liveSnapshot` in `db-local.ts` and `db-supabase.ts`; the cron step writing `live/{event}.json` and the daily copy; `GET /v1/live.json`; UPLOAD-API section | lifecycle test on a moved clock: test and practice runs are not counted, a `continue` does not double the calls, a group under 5 folds; the GET serves the stored copy; `check-sql.sh` smoke-calls the function on a throwaway Postgres | yes | copy the migration to the website repo, apply it, deploy the Worker (before 4 Nov to test on the dry run) |
| 3 | (b) `goalcup-verify --stats` with the de-dup rules | fixture traces in `tests/fixtures/m2-*`: Claude Code's repeated usage lines count once, Anthropic `message_start` plus cumulative `message_delta`, a replayed `tool_result` counts once, native and wire never add, "keep going" matched exactly, no text in the output | yes | none (an organizer tool, not in the participant release) |
| 4 | (b) `run_stats` table, `goal_cup_upload_stats_runs` and `…_record_trace_stats`, `GET /v1/stats/runs`, `PUT …/trace`; `scripts/stats-job.py` | Worker: wrong key `401`, stale `409`, unknown key `400`; script: reassembly from a fake bucket directory equals the original chain, a bad SHA-256 stops that run | yes | `STATS_KEY` secret, an R2 read-only token, the machine that runs the job and its timer |
| 5 | (c) the GitHub reader in the cron | stubbed `fetch` as in `test/github.test.ts`: `202` retried next round, outsiders' PRs not counted, pushes after the end left out, `403` ends the tick | yes | deploy (the token exists) |
| 6 | (d) `/goalcup/live` in `agentcommunity_page` | the page renders a fixture `live.json`, `null` blocks render as "not yet", a failed fetch keeps the last copy | yes | website deploy |

Steps 2 and 6 make a useful page on their own before 7 Nov. Steps 3 to 5 can land during the week:
the trace and GitHub blocks stay `null` until they do. The Worker deploys of steps 2, 4 and 5 should
happen before the 4 Nov feature stop, or wait for a quiet hour with the owner's go-ahead.

## Owner questions

1. **Anonymous or public by default?** RULES says anonymous by default; the website's `signups.anonymous`
   defaults to `false`. Not blocking here (all numbers are aggregate), but it decides whether any
   per-entry view can ever exist.
2. **Per-entry numbers:** never during the week (proposed). After results, for published runs that
   opt in only?
3. **Where may raw traces be read for stats?** The verify job copies every competition trace to the
   machine that runs it. RULES says traces are used for compliance review and the announced research
   project. Is a public aggregate within that, should the privacy notice name it, and should it count
   only runs with `research_opt_in`? Which machine: the VPS (`agentbrief`) or an organizer machine?
4. **Model mix:** hide it until the start window closes (8 Nov 00:00)?
5. **GitHub reader in the Worker's cron** (proposed) or a VPS collector like the events feed?
6. **Lines of code:** show "lines changed, including generated files", or leave it out?
7. **Recording problems and disk pauses:** keep them off the public page (proposed)?
8. **The dry run:** serve `live.json` for the dry-run event from now (a free test), and show it on the
   website marked "dry run", or keep the page hidden until 7 Nov?
9. **The 5-run floor:** is 5 right?
