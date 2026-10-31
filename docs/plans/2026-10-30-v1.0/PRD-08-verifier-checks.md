# PRD-08: Verifier and checks

Id: PRD-08. Version: goalcup 1.0. Status: plan for the build. Date: 30 October 2026.
Revision 2, 30 October 2026: acts on every PRD-08 item of the consistency check and reviews A and B, and
follows PRD-00 and PRD-01 revision 2 (section "Review answers" at the end). Base: main at 4b226de2.
Revision 3, 30 October 2026: alignment fixes against the ledger (section "Alignment answers" at the end).
Revision 4, 30 October 2026: carries ledger R3a, R3b and T4: the history check (section 3.2), the
checks against the harness's own log made optional, and the null mode and `goalcup recover` gaps.
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md).

## Goal

One small `goalcup-verify` that reads GCR1, proves a record is whole, and fails loudly on every loss
that run 1 hid. Its JSON report (seam S6) is what the pad (PRD-12), the watchers (PRD-16) and organizers read.
The proxy record is the proof (ledger T4): every check reads the record, the server copy and files the
pad hands it. No command opens a network connection or calls an outside API.

## Points served

5 (the server copy is proven whole and rebuilt by ID and time), 3 (no silent loss: a loss without a
GAP is a failure), 1 and 2 (the history check finds a missed model call from the record alone; where
the harness keeps session files, wire calls and subagents are also checked against its own log), 4
(the secret scan), 6 (the counts the participant saw equal the verifier's recount).

## Owner decisions it carries out

- S1 and L7: verifier and Worker move to the new format in the same build; no backward compatibility,
  no v5 reader (BRIEF3, 29 Oct evening). The run 1 converter is not built (PRD-00 "Not in this build").
- R4: loss can never be silent. Any loss leaves a gap with a start and an end (handover ask 3, BRIEF2).
- R6: count a model call only once its bytes are in the record; show how long recording was off
  (handover ask 4). The participant, status, heartbeat and server see only written counts.
- R5: no caps of our own: the old reader's caps (128 open streams, 128 connections) are cut.
- L6: research decoders are cut, except the model and usage parts PRD-09 keeps.
- L6: signing, receipts, acks, baseline and seal manifests, termination proof are cut (PRD-00 question 3).
- R8 and section 7: record raw, interpret later. Everything here runs after the run, or beside it in a
  watcher, in a sandbox box (ledger B1, section 1 "Where it runs"), never in the recorder (AGENTS.md, D3).
- R3a: a crash loop runs null mode (pass-through) and retries for ever. The next normal boot writes
  one `recorder_down` gap with cause `null_mode` for the whole span (PRD-02 section 8 step 9). The
  verifier names it, however long; it is never a `fail` (section 3, inv6).
- R3b: `goalcup recover` restarts only the recorder after the watchdog died; its boot writes one
  `recorder_down` gap with cause `watchdog_lost`. Named the same way. The run CA key kept in
  `rec/run-ca.key` during the run goes in the pad's `--known` file, so the scan proves it never
  reaches the record or the server copy.
- T4: the proxy record is the proof. This PRD supplies no unexplained gap (integrity and honesty
  checks), every subagent recorded, no secret, server copy whole, and the history check (section
  3.2), all offline. Usage stats present is PRD-09's (PRD-12 C5). The checks against the harness's
  own log (`wire_keeps_up`, `threads_match`, the manifest part of `subagents_present`) are an
  optional extra: they run only where the harness keeps session files, and their `skip` is never a
  failure (section 3.1). No provider usage lookup or other outside API is called.
- T8 and done item 1: Hermes passes on the history check (it resends the conversation to its
  model). Its optional log check runs per call when its `state.db` rows carry usable times, else by a
  per-session comparison after the seal (PRD-00 question 7, default; section 3.1).

## Today

| What | Where | Fact |
|---|---|---|
| Crate size | `crates/goalcup-verifier/` | 13,488 src lines, 20,724 test lines, 2,182 fixture lines (`wc -l`) |
| Three modes | `crates/goalcup-verifier/src/bin/goalcup-verify.rs:44-71` | `--capture-integrity`, `--research-json`, `--stats`, plus `--dev-key` for signing |
| Exit codes | `crates/goalcup-verifier/src/capture_analysis/reader.rs:269-281`; `goalcup-verify.rs:296-310` | 0 valid, 1 incomplete or unsealed, 2 invalid or torn tail, 3 no usable report (usage errors too). Checked 30 Oct |
| Gaps never fail | `reader.rs:1126-1141` | `gap_recorded`, `producer_recovered`, `producer_unclosed` only make the report "incomplete" (exit 1) |
| A hole needs a gap | `reader.rs:1726-1733` | `segment_source_unexplained_hole` unless a gap is pending on that stream: the one silent-loss rule today. Checked 30 Oct |
| Caps as verdicts | `reader.rs:78, 1384, 1566, 1694` | More than 128 goal streams, open connections or open streams is an error (exit 2). Checked 30 Oct: `MAX_GOAL_STREAMS` at `:78`, `connection_limit_exceeded` at `:1566`, `open_stream_limit_exceeded` at `:1694` |
| Cross-leg rule | `crates/goalcup-verifier/src/capture_analysis/report.rs:376-386` | Judges only zero against non-zero. `wire_last` exists (`session.rs:193-197`) but is never compared with the native leg |
| Provider errors | `crates/goalcup-verifier/src/capture_analysis/stats.rs:949-956` | "Models, provider errors and id-less calls come from the same source as the tokens". In run 1 that was native, so the wire's rate-limit errors read 0. Checked 30 Oct |
| Double count trap | `crates/goalcup-verifier/src/capture_analysis/openai.rs:475-486, 545-558` | A `response.failed` error object and an `error` stream event each make an error observation. Checked 30 Oct |
| Evidence label for judges | `report.rs:391-401`; `docs/RULES.md:306-319` | `wire_and_native`, `wire_only`, `wire_undecoded`, `native_only`, `none` |
| Producer lineage | `crates/goalcup-verifier/src/capture_analysis/lineage.rs` (142 lines) | Recovery supersession of proxy producers. Producers do not exist in 1.0 |
| Who uses it | appendix C section 8.2 | Nothing on the recording path. `goalcup doctor` (`crates/goalcup-cli/src/commands/doctor.rs:246-316`, crate link at `crates/goalcup-cli/Cargo.toml:33`), `scripts/matrix.sh:2054-2105`, `scripts/linux-harness-smoke.sh:231`, the organizer after `scripts/fetch-run.sh` |

What run 1's server copy got from today's verifier (run report, "Server copy", section 1): every mode
exit 1, never 2. `cross_leg.state` was `consistent` although the wire stopped at 15:43:13Z and the
native leg ran to 16:54:43Z. `provider_errors.total` was 0 although the chain holds about 97
`rate_limit_exceeded` error events and 99 `response.failed` messages, and Codex saw 95 calls fail
(run report measure 9). PRD-00 fixes the one range all PRDs use: 95 to 99. Only `producer_unclosed`
hinted at the loss. The 23 gaps had one time each and `lost_bytes: null`. The server stored
`model_requests` 1,766 against 842 calls in the record.

## Design

### 1. One binary, four subcommands

`crates/goalcup-verify`, binary `goalcup-verify`, written fresh against `goalcup_format` (F1). It links
`goalcup-format` and its own `usage` module (PRD-09). It never links the recorder crate
(AGENTS.md dependency rule, "the recorder never links the verifier" in both directions).

| Command | Does | Used by |
|---|---|---|
| `check <dir>` | Integrity, honesty and count checks on an entry folder, a run folder (all entries in order) or a fetched server copy. Text to stderr, `--json` report (S6) to stdout | Pad, watchers, organizers, participant |
| `check --server <fetched> [--local <entry dir>]` | Also proves the server copy whole, equal to the local part files, and its database row equal to the seal | Pad, organizers |
| `check --live <entry dir>` | The same, on an entry still recording: reads to the durable point (last index line), accepts the open frame, and also checks that the recorder wrote a COUNT recently | Watchers during long runs |
| `dump <dir>` | One JSON line per record (PRD-01 `dump`); `--sid`, `--kind`, `--text` filters | Builders, watchers |
| `secrets <path>... [--known <file>]` | The secret scan. A path that is an entry, run or fetched folder is decoded as a record; any other file or folder is scanned as raw bytes with the same known-value forms and shapes (no MASK rules there); `-` reads stdin, so a caller can pipe `git log -p --all`. Reports counts and hash prefixes only | Pad, organizers |
| `usage <dir>` | PRD-09's usage reader; this crate only dispatches to it | Pad, watchers, organizer job |

Where it runs (ledger B1): every `goalcup-verify` command runs in a box, never on the conductor.
`check --live` runs in the pad box via `sandbox exec`, under `nice -n 19`, so it does not slow the
harness. `check --server`, `usage` and `secrets --known` run in the run's pad box (before it is
removed) or a fresh check box, on the fetched folder the conductor copies in. The known-values file
copied there holds only values that entered that run's box (harness key, gateway key, push token,
planted values, canary, and the run CA private key from `rec/run-ca.key`, ledger R3b), 0600, deleted
after the scan. The conductor runs only `fetch-run`, sandbox
control and git.

The judge view (`goalcup-verify judge`) is not in this build (PRD-00 "Not in this build", review B N2).
Its one fact judges need now, the evidence label, is a key in S6 (section 5).

Inputs `check` takes:

- The record itself, always, and nothing else is needed. The history check (section 3.2) reads only
  the recorded requests and responses. Native session files, where the harness keeps them, are in
  the record too (PRD-04), so the optional comparison with the harness's own log is read from the
  record, not from the disk. That makes every check work on the server copy.
- `--harness-files <manifest.json>` (pad only, optional): the pad lists files on disk under the connector's
  native roots, as `[{"path": "<source>:<relative path>", "size": n}]`. The check then also proves
  that every file on disk has a stream. The verifier never walks a home folder itself.
- `--known <file>` (pad only, 0600): real and planted secret values, one per line, for the scan.

Exit codes keep today's meaning (`reader.rs:269-281`): **0** whole, every check passed; **1** honest
but incomplete: every loss is named by a GAP, an off span or a named tunnel, or the entry is unsealed
or live, or local part files were pruned; **2** a failure: a broken invariant or a silent loss; **3**
no usable report: the input cannot be read, or the command line is wrong. `--help` and `--version`
exit 0. Copy `goalcup-verify.rs:296-310` for the parse rule and `:264-294` for safe output files (new
file, 0600, outside every input folder).

### 2. The reader

One streaming pass over the record stream: concatenated part files (or the fetched `rec.gcr.zst`),
decoded with a 2^27 window (`goalcup_format::read`). Never decode a chunk alone. No caps of our own.
State per open stream is a few words (kind, conn, req, next offset, open gap span, last time, a 4 KiB
scan tail), dropped at END. Per thread and per 10-minute bucket, a few counters. U1's call table
(PRD-09) holds one small row per call: about 20 MB for a busy week (estimate). Memory is the 128 MiB
window plus that. No memory guard: this is an offline tool, and a week with thousands of subagents
holds at most a few thousand open streams at once.

Boots replace producers. `lineage.rs` goes. Boot order is invariant 6.

A finding is a code plus counts, never bytes. Distinct finding codes are kept as in `reader.rs:442-476`
(copy the idea, not the 128 cap). Each check keeps its first failure with raw offset, `sid` and wall
time, so a person can `dump --sid` to it.

If PRD-01's crash-recovery fallback is used (part files decoded one by one, PRD-01 section 12), the
reader decodes per part file. `goalcup_format::read` hides which; this crate does not care.

### 3. The checks

Every check has one `id`, one pass rule and a result: `pass`, `named` (loss present, and every lost
part sits inside a named GAP, off span or named tunnel), `warn`, `skip` (with why), or `fail`. `fail`
gives exit 2, `named` gives exit 1. `warn` and `skip` never change the exit.

**Integrity: PRD-01 section 13, all 14 invariants.** One check each. Any failure is exit 2. Ids keep
the names other PRDs already read:

| Invariant | Id | Notes |
|---|---|---|
| 1 to 9 | `inv1` to `inv9` | inv4 counts from `meta.from` and exempts `conn` streams; inv6 is boot order (one `recorder_down` per boot after 0); inv7 is "every COUNT equals a recount". A `recorder_down` gap with cause `null_mode` (one gap for a whole crash-loop span, ledger R3a) or `watchdog_lost` (a boot started by `goalcup recover`, R3b) is a named loss of any length, never `fail`; the text output prints its cause and length |
| 10 | `streams_closed` | In an unsealed or live entry, open streams are listed and the result is `named`, not `fail` |
| 11 | `seen_matches_written` | A stream with an unknown-loss GAP is `named` with the hole's size, not checked |
| 12 | `gap_has_end` | A GAP with start equal to end and `lost` 0 or 1 is run 1's shape: `fail` |
| 13 | `recorder_alive` | COUNT at most 180 s apart within a boot, after taking off `suspend` time. With `--live`, also: the last COUNT is under 180 s old by the wall clock now, minus any sleep the record shows. A stale COUNT while the entry's `status.json` says `mode: "null"` is `named` "null mode since T" (R3a: the next normal boot writes the gap); any other stale COUNT is `fail`, the alarm that the recorder and its watchdog are both gone (the way back is `goalcup recover`, R3b) |
| 14 | `inv14` | Native resume has no hole across boots |

Also exit 2: a torn tail (a record cut inside its length) at the end of a sealed entry; two streams of
one boot with the same native `path` and `gen` (PRD-04 interfaces row). A torn tail at the end of a
live or crashed entry is the open end PRD-01 section 12 expects. GAP reasons 8 and 9 (reserved,
never written by 1.0) and any unknown number show as `other(n)` with `warn` (`gap_reason_unknown`).

**Honesty: the checks that must fail run 1's shape.**

| Id | Rule | Pass | Run 1's shape fails because |
|---|---|---|---|
| `history_complete` | The history check, offline on the record (section 3.2). Every earlier assistant answer found inside a later request must exist as a recorded response. A missing one is a finding with its thread. `named` when a named GAP, off span or named tunnel covers the time it was made; `skip`, with why, where requests carry only a previous-response id or after context compaction | No answer in a resent conversation that was never recorded as its own response | The synthetic record drops one root-thread call in the middle with no GAP while later requests resend its answer (section "Automated tests"). A wire that stops for good, as run 1's did, leaves no later request to show it: `wire_keeps_up` and the integrity checks catch that |
| `wire_keeps_up` | Optional (section 3.1): runs only where the harness keeps session files. Harness calls (native leg, U1 rows with times) against wire calls whose outcome is not an error, cumulative over time. Fail when, at any moment T, harness calls up to T exceed wire calls up to T plus 2 minutes by more than max(3, 2%), and the excess is not inside a named GAP, off span, a named tunnel (GAP 1 for `hello_too_big`, 6, 7, 13 or 14) to the harness's model host, or a `conn` GAP (for example 11) on a decrypted connection to it. A harness call is covered when its time lies from the GAP's `start_wall` to its `end_wall` plus the 2-minute lag. A GAP that fails `gap_has_end` covers nothing. Also fail when the harness's last call is more than 10 minutes after the wire's last call with no such cover (`wire_ended_before_native`). Wire above harness is normal (retries, warm-ups) and only reported. `skip` when U1 has no per-call times for this connector (section 3.1) | Never harness above wire, unexplained | Wire stops 15:43:13Z; 473 harness calls follow to 16:54Z, 71 minutes, no cover |
| `threads_match` | Optional, as `wire_keeps_up`. Per thread (U1 thread ids: Codex `thread-id` header and `client_metadata`, Claude Code subagent files, and so on): harness calls not above non-error wire calls, by the same tolerance. A thread with harness calls and no wire call fails, unless covered as above. A wire-only thread is reported. For a harness whose log has totals only, the totals form of section 3.1 | Every thread matches or is explained | Root thread 358 wire against 591; 15 threads have harness calls and no wire call |
| `subagents_present` | Every child a parent names (U1 `children`: Codex `receiver_thread_ids`, `crates/goalcup-verifier/src/capture_analysis/native/codex.rs:418-427`; Claude Code `subagents/agent-*.jsonl`; Hermes `sessions.parent_session_id`) has a native stream or wire calls, or sits inside a named gap. With `--harness-files`: every append file on disk has a `native` stream whose last END `seen` equals its size, or a named GAP; every database or snapshot file has a `snapshot` stream (sizes not compared: a database keeps growing) | Every subagent has a leg | 11 children have neither leg; 11 rollout files have no native stream |
| `listed_hosts_decrypted` | Every `conn` to a host on a list (`listed` not null) is `decrypt`; or a `sign_in` tunnel (no loss, `pass`); or a tunnel whose GAP names its reason (1 `memory_guard` for `hello_too_big`, 6, 7, 13, 14: `named`, PRD-00 ruling, PRD-01 section 4). A decrypted `conn` with a `conn` GAP after a parser fault (4, 11 or 1) is `named`. A listed tunnel with no GAP is `fail`. Two `warn` facts: a `long_tunnel` note on an unlisted host (a possible model host to add), and `refusal_after_trust`: a host and ClientHello `fingerprint` pair that completed a decrypted connection earlier in the entry and later got a `tls_refused` tunnel (review A finding 1: the harness itself may be tunnelled by a learned refusal). A `tls_refused` GAP with tail `{"alert": false}` is skipped by `refusal_after_trust`: nothing was learned from it (PRD-03 section 1) | No listed host relayed blind | Passes (no model host was tunnelled in run 1) |
| `conn_bytes` | Per decrypted `conn`, each direction: `plain_up` or `plain_down` (PRD-01 END tail) against what its streams account for: request and response heads (the OPEN `headers` size, approximate), HTTP body bytes on the wire (END tail `wire`), WebSocket message bytes, and any `conn` GAP's `lost - 1`. `warn` when more than 10% and more than 1 MiB is unaccounted. `fail` when more than 50% and more than 1 MiB is unaccounted and no `conn` GAP covers the connection (review A finding 2: with PRD-03's `conn` GAP after a parser fault, unaccounted bytes on that scale can only be a silent loss) | Bytes add up within the stated slack | Not applicable to run 1 (v5 had no such tails); it guards 1.0's reused-connection fault |
| `mask_failclosed` | `warn` when one stream has more than 1 MiB of `fail_closed` MASK bytes, with the stream count and the total (COUNT `mask_failclosed_bytes`, review A finding 3) | No large fail-closed span | Not applicable |

A note on inflate: WebSocket messages are recorded inflated, so a deflate connection can account for
more bytes than went over it. That only makes `conn_bytes` quieter; it never fails a good run.

#### 3.1 Which harnesses have a per-call log (the optional checks)

`wire_keeps_up` and `threads_match` compare the wire with the harness's own log. Ledger T4 makes
them an optional extra check: they run only where the harness keeps local session files (native
streams in the record) and U1 can read them. Everywhere else they are `skip` "no session files",
and that `skip` is never a failure: the history check (section 3.2) and the integrity checks carry
the proof. The manifest part of `subagents_present` (`--harness-files`) is optional the same way.

U1 (PRD-09) says per connector what its native leg can give: per-call rows with times, per-session
totals, or nothing (`readable: false`). This crate does not guess.

`harness_log` comes from U1: `readable: false` gives `none`; `native_form` from PRD-09 gives
`per_call` or `totals` (Hermes: `per_call` when its assistant `messages` rows carry a usable
`timestamp`, else `totals` from per-session `api_call_count`). In the totals form, `threads_match`
compares per session, or per entry (all `ok` and refused wire Chat or Messages calls of the entry
against the sum of its sessions' `api_call_count`) when wire calls carry no session id. For a harness
whose wire thread comes from PRD-09's join (Claude Code, Hermes), `threads_match` reads per-thread
`native_only` against the tolerance. Harness calls after a thread's `native_until` are not compared.

| Native leg in 1.0 | Harnesses | `wire_keeps_up` | `threads_match` |
|---|---|---|---|
| Per-call rows with times (`per_call`) | Codex, Claude Code (PRD-09 section 5, measured or fixtures); Hermes when its `messages` rows carry a usable `timestamp` | runs | runs per thread |
| Per-session totals, read after the seal (`totals`) | Hermes when its rows carry no usable time (`state.db` `api_call_count`); others once a pad sample adds a reader | `skip` "no per-call times" | totals form: per session, wire non-error calls for that session against the session's own count, same tolerance, after the seal. When wire calls carry no session id, per entry, as above |
| None (`readable: false`), or an unreadable database copy (review B A7) | every other harness until a pad sample | `skip` "the harness's own log is not read" | `skip` with the same words |

`skip` is never `pass`. If U1 reads zero harness calls while the wire has calls, both checks are
`skip` with "could not read the harness's own log". A `skip` here never makes the pad's C1
INCONCLUSIVE on its own: C1 rests on `history_complete` (PRD-12 interfaces row below).

#### 3.2 The history check (`history_complete`)

Most harnesses resend the whole conversation with every model call: Chat Completions `messages`,
Anthropic Messages `messages`, Gemini `contents`, and Responses `input` when the request carries no
previous-response id. So every answer the model gave earlier shows up again inside later requests.
If an answer is in a later request but was never recorded as a response, the call that produced it
was missed. This needs only the record: no session file, no network, no provider lookup.

How it works, in the same one streaming pass (`src/history.rs`):

1. **Answers recorded.** For every recorded model response (U1 pairs request and reply, PRD-09),
   take its answer keys: each tool-call id (`toolu_…`, `call_…`, Responses `call_id`) and the
   SHA-256 of its text, whitespace-trimmed and collapsed. For a streamed response the text is the
   joined `delta`, `text` deltas (the same joiner as the secret scan). Keys go in one set for the
   whole input; a few bytes per answer.
2. **Answers resent.** For every decoded model request, list its earlier assistant turns (role
   `assistant`, Gemini `model`, Responses `message` and `function_call` items) with the same keys.
   Reasoning items and turns with no id and no text are not checked.
3. **Threads.** A request joins the open chain whose last list it extends (its earlier turns start
   with that list); else it starts a new chain. U1's thread id, where the wire carries one, names the
   chain in the report. Only the turns new since the previous request of the chain are kept, so
   memory grows with answers, not with history length.
4. **Verdict per new turn**, after the pass, against the whole set (so an answer copied from another
   thread or an earlier entry of the run still matches):
   - found: fine;
   - missing, with a tool-call id: a missed call. `named` when a named GAP, off span (including
     `null_mode` and `watchdog_lost`) or named tunnel to that model host overlaps the time between
     the chain's previous request and this one; else `fail`;
   - missing, text only: `warn` `history_text_unmatched` (a harness may trim or rewrite answer text);
     PRD-16 may make it `fail` once R2 and R6 show the text rule holds;
   - missing in a chain's first request: `warn` `history_unanchored` (from before the record, a
     resumed session, or a summary), never `fail`.
5. **Skips, always said.** A request that carries only a previous-response id (`previous_response_id`
   with no resent turns) adds nothing; a chain made only of such requests is `skip` "history held by
   the provider". Context compaction: when a request with the same U1 thread id no longer extends its
   chain, the rest of that thread is `skip` "history rewritten at T (compaction)". A request whose
   body is holed or not decoded is skipped and counted. If no chain could be checked, the check is
   `skip` with the reason (also for the dodger case: nothing to its model host was decrypted).

Each finding names the thread (U1 thread id or chain number), the request's `sid` and wall time, and
the missing turn's position and kind (tool call or text), never its bytes. Facts: answers checked,
missing by kind, requests skipped by reason, threads compacted.

Run 1's shape is not this check's to catch: when the wire stops for good, no later request resends
the lost answers. The check catches a call dropped in the middle while later calls are recorded,
for every harness that resends history, whether or not it keeps session files.

**Reports that never fail on their own.**

| Id | What | Why |
|---|---|---|
| `provider_errors` | Wire calls whose U1 outcome is an error (`RateLimited` or `Error`), by code and HTTP status. One per call: U1 already folds an `error` stream event and a `response.failed` of one call into one row. Native-leg errors are listed apart | Run 1's shape must read the errors it holds, not 0 and not double. They also decide which wire calls `wire_keeps_up` and `threads_match` count |
| `counts` | The recount: model requests written (`http_post` + `ws_up_msgs`, PRD-01 section 9), `hosts`, `tunnel_hosts`, whole and holed requests, gaps and lost bytes by reason, `after_stop` bytes, `mask_failclosed_bytes`, off time, tunnels, native files and bytes, per boot and per entry. A boot's total is its last COUNT; for a dead boot, the COUNT in its recovery frame (PRD-01 section 9). Equal to every COUNT (that is `inv7`) and to the seal totals (`counts_match_seal`, exit 2 if not) | The number the participant saw is the one the verifier recounts (PRD-00 done item 1) |
| `bytes` | Raw bytes, compressed bytes, and framing bytes (raw minus DATA payload), per entry | Replaces `scripts/research-measure.py` (PRD-13) |
| `timeline` | Per 10 minutes: harness calls, wire calls, wire errors, native bytes, gaps, off time | Run 1's reconcile was read this way (appendix M section 5.6); PRD-16 X1 reads it |

The `after_stop` GAP (15) is a named loss (exit 1), never off time. A `source_changed` END with no GAP
is allowed when PRD-01 section 3 says so.

**Server copy** (`check --server`), read from PRD-07's fetched layout (seam S13).

| Id | Rule | Result |
|---|---|---|
| `server_chunks_whole` | From `fetch.json`: offsets run on from 0; link `seq` runs on; each chunk's `prev` equals the previous chunk's `head`; the chunk heads equal the LINK hashes at those `seq` in the decoded record (inv2 recomputes them); the last chunk ends at the final heartbeat's `seq` and head. Each slice of `rec.gcr.zst`, located by the chunk's `bytes` in `fetch.json`, is hashed again against its SHA-256. When `fetch.json` has `final` null, it checks continuity only and reports the open end as incomplete (exit 1, never `fail`) | `fail` on any break |
| `server_equals_local` | With `--local` and part files kept (`GOALCUP_KEEP_PARTS=1`, start event `keep: true`): SHA-256 of `rec.gcr.zst` equals that of the concatenated local part files, up to the shorter of the two | `fail` on difference. Pruned local part files: `skip` "use the server copy" (PRD-12 maps it to FAIL) |
| `server_counts_match` | The entry row in `fetch.json`: `model_requests` equals the seal's written total; `upload_offset` and `final_seq` equal the last chunk; the last heartbeat's `recording` equals what PRD-06's rule (PRD-06 section 6) gives from the seal totals: `ok` only with zero gaps, lost bytes and off time; and `fetch.json` `counts` (the last heartbeat's) equal the seal totals key by key (`model_requests`, `gaps`, `lost_bytes`, `off_ms`). When `fetch.json` has `final` null: `skip` "not sealed" | `fail` on difference |

**Secrets** (`secrets`, and inside `check` when `--known` is given).

- Per stream, DATA pieces are joined in offset order with a 4 KiB overlap, so a secret split across
  two records is found (run 1's observer missed this, appendix M section 5.6 lesson 3). Meta values
  and event JSON are scanned too.
- Known values (`--known`): raw, base64 in all three alignments, percent-encoded, JSON-escaped.
- Shapes: PRD-05's `crates/goalcup/src/mask/shapes.tsv` (seam S12: kind, guard, regex), read with
  `include_str!` and compiled **without** the left guard, so a key glued to a letter is reported even
  though the masker accepts missing it (ledger M1).
- Deltas: for SSE and `ws_down` JSON text, the string values of `delta`, `text` and `partial_json` are
  joined in order per stream and scanned through the same 4 KiB tail, because a key the model repeats
  across deltas is not masked (PRD-05 risks).
- Masks: a MASK span over 16 KiB is a `fail` unless its rule is `fail_closed` (PRD-05 interfaces row
  PRD-08); `fail_closed` spans are reported by bytes. Spans are counted by rule.
- Output: hits by kind and stream, with the first 12 hex digits of each hit's SHA-256 (run 1's
  method). Never the value. Any hit of a known value is `fail`. A shape hit is `fail`, except a hit of
  a guarded rule whose first byte follows a letter, digit or `_` (the left guard M1 accepts, PRD-05
  section 4): that is `warn` `glued_shape`, with its kind, stream and hash prefix.
- Raw inputs (a path that is not a record, or `-` for stdin): the same known-value forms and shapes over
  the bytes as they are, with the same 4 KiB overlap; no MASK rules apply there. About 30 lines in
  `secrets.rs`, reusing the same matcher.

### 4. Seam S6: the `check --json` report

Stable by H+3 with real `inv*` results and `skip` for the rest. The pad and the watchers read only
these keys; unknown keys are ignored by readers.

```json
{"verifier":"1.0.0","input":"local|server|live","run":"r…",
 "result":"pass|named|fail","exit":1,
 "entries":[{"entry":"e…","sealed":true,"seal_reason":"harness_exit","boots":2,
   "connector":"codex-cli","label":"wire_and_native","harness_log":"per_call|totals|none",
   "bytes":{"raw":0,"compressed":0,"framing":0},"links":0,
   "checks":[{"id":"wire_keeps_up","inv":null,"result":"fail",
              "why":"harness above wire by 473 calls from 15:43:13Z to 16:54:43Z (71 min)",
              "first":{"o":123,"sid":0,"wall":"…"},"facts":{}}],
   "counts":{"model_requests":914,"req_whole":0,"req_holes":0,"gaps":{"memory_guard":0},
             "lost_bytes":0,"lost_unknown":0,"after_stop_bytes":0,"mask_failclosed_bytes":0,
             "off_ms":0,"tunnels":0,"native_files":0,"threads":0},
   "provider_errors":{"wire":{"rate_limit_exceeded":97},"native":{}},
   "history":{"checked":1204,"missing_call":1,"missing_text":0,"unanchored":0,
              "skipped":{"previous_response_id":0,"compacted":0,"undecoded":0}},
   "threads":[{"thread":"…","parent":"…","harness":591,"wire_ok":358,"wire_err":40,"result":"fail",
               "history":"fail"}]}],
 "server":{"chunks":20,"whole":true,"equals_local":null,"db_match":false}}
```

The numbers are an illustration of run 1's shape, not run 1's measured figures. `why` is plain
English, one line, no secret, no path outside the record. The text form on stderr prints `why` per
failed or named check, then one coverage line (section 5).

### 5. The label and the coverage line (instead of the judge view)

The full judge page waits until after the merge. Two small things stay, because RULES and the
CHEAT-CHECKLIST cite them (PRD-15 rewrites that text) and the pad prints them:

- **`label`** in S6: `wire_and_native`, `wire_only`, `native_only` or `none`, from written model
  requests and native streams with bytes. `wire_undecoded` goes: whether PRD-09 could read a body is
  `harness_log` and the U1 facts, not the label. The label is a hint for judges, not a judgement.
- **One coverage line** in the text output, for example: "Recorded: 914 requests, 97 refused by the
  provider. History check: 1,204 answers checked in 43 threads, 1 missed call. The harness's own
  log: 1,261 calls; harness calls with no recording: 473, from 15:43 to 16:54 (71 min). Recording
  off: 0 times, 0 s." The seconds are the `off_ms` total. When no harness reader exists for the
  connector: "The harness's own log is not read for this harness (optional)"; when the history
  check is skipped, its reason. Never "everything recorded".

### 6. What of `crates/goalcup-verifier` stays and goes

The whole crate is deleted by PRD-13 after this PRD's tests pass. Nothing is kept as code except what
is copied below.

| Part | Lines (src / tests) | Fate |
|---|---|---|
| `bin/goalcup-verify.rs` | 326 / 0 | Copy `create_output_file` (`:264-294`) and the parse rule (`:296-310`). Cut the three flag modes and `--dev-key` (`:44-110`) |
| `lib.rs` | 53 / 0 | Cut: `SERVER_PUBLIC_KEYS` (`:34-35`) and `VerifyOptions` (`:39-53`) go with signing |
| `capture_analysis/reader.rs` | 1,889 + 181 inline / 3,737 (`tests/it/m2_capture_integrity.rs`) | Keep the ideas, rewrite for GCR1: one streaming pass, distinct finding codes (`:442-476`), first failure, exit mapping (`:269-281`), the hole rule (`:1726-1733`, now invariant 4). Cut: receipt and ack checks (`:620-665`, `:925-975`), baseline and seal manifests and Git reads (`:985-1110`), launch goal rules (`:1115-1125`, `:1818-1889`), producer admission and recovery (`:1378-1660`), the caps (`:78`, `:1566`, `:1694`) |
| `capture_analysis/lineage.rs` | 101 + 41 / 0 | Cut. Producers and recoveries are gone; boot order replaces them |
| Decoders: `anthropic.rs` 1,638, `openai.rs` 664, `chat.rs` 1,080, `thread.rs` 383, `native/*` 1,279, `wire.rs` 1,339, `tracker.rs` 600 | 6,983 / about 11,000 | PRD-09 copies only the model, usage, error and thread parts (for example `openai.rs:470-486, 545-558`; `native/codex.rs:125-128, 286-308, 418-427`). Everything else cut |
| Framework: `api.rs` 313, `registry.rs` 213, `session.rs` 940, `operations.rs` 348 | 1,814 / 1,253 | Cut. `cross_leg` is replaced by `wire_keeps_up`; the label moves to S6 |
| `report.rs` | 946 / 2,456 | Cut |
| `stats.rs` | 1,115 / 873 | Cut. The one-error-per-call rule moves to PRD-09 (U1) |
| `tests/it/v5_corpus.rs` and `tests/fixtures/v5-corpus/` | 0 / 718 plus 912 fixture lines | Cut |
| Other tests and fixtures | rest of 20,724 / 2,182 | Cut. PRD-17 reads `m2_capture_integrity.rs` for lessons; PRD-09 decides which `m2-*` and `connectors` fixtures to copy |

Couplings that go with the crate: `crates/goalcup-cli/Cargo.toml:33` and the doctor decoder check
(`doctor.rs:246-316`) go with `goalcup-cli` (PRD-13); `scripts/matrix.sh`, `research-replay.py` and
`linux-harness-smoke.sh` callers go to PRD-13 and PRD-14; the pad replaces them.

## Files it owns

Create (all new, in the PRD-00 tree):

- `crates/goalcup-verify/Cargo.toml`: every dependency declared once in phase 0, for PRD-09 too:
  `goalcup-format`, `zstd`, `serde`, `serde_json`, `sha2`, `clap`, `regex`, `base64`, and `rusqlite`
  with the `bundled` feature (PRD-00 ruling; PRD-09 needs it for Hermes). Nothing async, no TLS, no
  `test-hooks` feature (no check here needs one).
- `crates/goalcup-verify/src/main.rs`: arguments, dispatch (including `usage`, which calls PRD-09's
  `usage::run`), exit codes, safe output files.
- `src/lib.rs` (module list), `src/input.rs` (entry, run, fetched and live inputs; part file order;
  `fetch.json`), `src/integrity.rs` (the 14 invariants, boots, torn tail), `src/honesty.rs`
  (`listed_hosts_decrypted`, `conn_bytes`, `mask_failclosed`), `src/legs.rs` (`wire_keeps_up`,
  `threads_match` with its totals form, `subagents_present`, `provider_errors`, `timeline`, `label`),
  `src/history.rs` (`history_complete`, section 3.2), `src/server.rs`, `src/secrets.rs`,
  `src/report.rs` (S6, text and the coverage line).
- `crates/goalcup-verify/tests/check_*.rs`, `tests/server_*.rs`, `tests/secrets_*.rs`,
  `tests/common/` (record builders over `goalcup_format::PartWriter` with `window_log` 20, the run 1
  shape builder that PRD-09 and PRD-12 K2 also use).

Change: none outside the crate. Delete: none (PRD-13 deletes the old crate).

Must not touch: `crates/goalcup-verify/src/usage/`, `tests/usage_*.rs` and `tests/usage_fixtures/`
(PRD-09), `crates/goalcup-format/` (PRD-01), `crates/goalcup/` including `src/mask/shapes.tsv`
(PRD-05; read only), `server/` and `scripts/fetch-run.sh` (PRD-07), `scripts/pad/` (PRD-12),
`docs/RULES.md` and `docs/CHEAT-CHECKLIST.md` (PRD-15), the old crates (PRD-13).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 | F1: `read` over part files and `rec.gcr.zst` with an open end, `dump`, record types, the link check, `read_index` for `--live`. Invariants 1 to 14; GAP reasons 1 to 15 (8 and 9 reserved, shown as `other(n)`); the `conn` GAP rules; END tails `seen`, `up`, `down`, `plain_up`, `plain_down`, `wire`, `unsent`; COUNT keys incl. `hosts`, `tunnel_hosts`, `mask_failclosed_bytes`; `suspend` events; `start` keys `hosts` (with kinds), `connector`, `keep` | Used here |
| PRD-02 | GAP tail keys `guard`, `limit`, `cause`, `prev`, `exits`, `last`, `conns`, `hosts`; exit codes 70, 73, 74, 75 as `recorder_down` causes (shown in the text output); causes `null_mode` (section 8 step 9, ledger R3a) and `watchdog_lost` (`goalcup recover`, section 8.1, R3b); `status.json` `mode: "null"` for `--live`; `rec/run-ca.key` (its value goes in the pad's `--known` file); COUNT every 60 s. No test hook is used here | Used here |
| PRD-03 | `conn` meta `listed`, `kind`, `mode`, `reason` (incl. `hello_too_big`), `fingerprint`; the `conn` GAP after a parser fault; notes `long_tunnel`, `parse_stopped`; the `tls_refused` GAP tail `{"alert": false}`. Every named tunnel on a listed host (GAP 1 for `hello_too_big`, 6, 7, 13, 14) and every `conn` GAP after a parser fault (for example 11) is `named`; `sign_in` is `pass` | Used here |
| PRD-04 | Native meta `path`, `gen`, `from`, `fid`; snapshot meta `set`, `form`, `harness_exited`; END `source_changed` with no GAP allowed; GAP tail `unread_at_least`; one path and `gen` per boot; periodic database copies (the last whole `set` is the one U1 reads) | Used here |
| PRD-05 | S12: `src/mask/shapes.tsv` (kind, guard, regex), read with `include_str!` and compiled without the guard; rule kinds in `mask_rules`; only `fail_closed` spans may exceed 16 KiB | Used here |
| PRD-06 | `upload.json` `keep`; `GOALCUP_KEEP_PARTS=1` for `server_equals_local`; the `recording` rule from counts, which `server_counts_match` recomputes | Used here |
| PRD-07 | S13: `<outdir>/<entry>/rec.gcr.zst` and `fetch.json` (chunk list with keys and SHA-256, and the entry row: `model_requests`, `upload_offset`, `final_seq`, final head, the last heartbeat's `recording`, sealed time), each chunk's compressed size (`bytes`) and the last heartbeat's `counts` | Used here |
| PRD-09 | U1, owned by PRD-09 and used as is: `usage::Calls::new(&Start)`, `push(&Record)`, `finish() -> CallTable` with rows (leg, entry, boot, sid or path, thread, parent, wall times, kind, outcome incl. `RateLimited`, `Cancelled`, `whole`, model) plus `threads`, `readable`, `children`; `native_form`, per-session `api_call_count`, and per thread `native_only`, `native_until`, `readable`, `reason`. Also `usage::run(args) -> ExitCode`. The history check uses U1's request and reply pairing and wire thread ids; it reads the turn lists itself (`src/history.rs`), so U1 needs no change | Used here |
| PRD-10 | The `goalcup-verify check` line goes to `goalcup.log` only, when local part files are whole; otherwise it names the server copy. The participant's counts are the ones `counts_match_seal` recounts | Used there |
| PRD-11 | Connector name and the `start` event `hosts` decide the harness reader and `listed_hosts_decrypted` | Used here |
| PRD-12 | S6; the optional `--harness-files` manifest and the `--known` file (with the run CA key, R3b) the pad writes and copies into the box where the checks run (section 1 "Where it runs"). C1 = `history_complete`, plus `wire_keeps_up` and `threads_match` as an optional extra where the harness keeps session files (ledger T4; their `skip` is never INCONCLUSIVE; C1 is INCONCLUSIVE only when `history_complete` and both optional checks are `skip`); C2 = `subagents_present`; C3 = `secrets`; C4 = integrity checks and `server_*`. Dodger case: exit 1, from the named `pinned` tunnels (`history_complete` is `skip` "nothing decrypted", `wire_keeps_up` is `named` where it runs, `skip` where U1 has no per-call times). K2 uses this crate's run 1 shape builder | Used there |
| PRD-13 | Deletes `crates/goalcup-verifier/` after this PRD's tests pass. Size numbers that replaced `research-measure.py` are S6 `bytes` | Done there |
| PRD-14 | `cargo nextest` covers `crates/goalcup-verify` in `gate fast`; the release ships `goalcup-verify`. Test list: the seven files under Acceptance (no `judge_view.rs`) | Used there |
| PRD-15 | RULES "What a label is worth" and the CHEAT-CHECKLIST cross-leg text rewritten for S6 `label` and the coverage line; the judge view is "after the merge" | Written there |
| PRD-16 | The `history_text_unmatched` default (`warn`) may become `fail` after R2 and R6. Watchers run `check --live` every 5 minutes (hourly for a week-long entry) in the pad box via `sandbox exec` under `nice -n 19`, and `check --server` after each run in the run's pad box or a fresh check box (ledger B1); a `fail` goes in the night log. Tolerances as stated here until PRD-16 changes them with evidence | Used there |
| PRD-17 | H10 and K15 of PRD-17 write a part file through the real pipeline; their test also runs `goalcup-verify check` on it and asserts exit 0. No other lessons test writes one | Used there |

## What is cut

| Path | Lines (src / tests) | Tests that go |
|---|---|---|
| `crates/goalcup-verifier/` (whole crate, section 6) | 13,488 / 20,724, plus 2,182 fixture lines | All of `tests/` |
| Flags `--capture-integrity`, `--research-json`, `--stats`, `--dev-key` | in the above | `v5_corpus.rs` exit-code cases |
| The research report NDJSON format (`docs/research/m2-report-format.md` is PRD-15's to retire) | in the above | `m2_research_report.rs`, byte goldens in `tests/fixtures/m2-report-bytes/` |
| Not built in 1.0: the judge view, the run 1 converter, the silent-drop hook proof | 0 | none written |

New code: about 1,600 src lines (input 150, integrity 380, honesty 150, legs 300, history 170,
server 130, secrets 230 with raw inputs, report and main 100) and about 1,250 test lines. PRD-00
budgets 1,500; the judge view's 150 is out, the totals form, `conn_bytes` fail rule and `fetch.json`
reading are in, and ledger T4's history check adds about 170 (over budget by about 100; the replan
agent may weigh it, B2a).

## Acceptance

### Real-run proofs (first)

| # | Proof | Pass |
|---|---|---|
| R1 | **Run 1's shape fails, on the synthetic record and on 1.0 itself.** (a) `check_run1_shape.rs`'s record written as a part folder and as a fake fetched copy (PRD-12 K2). (b) A1, the 2-hour Codex run on 1.0 (PRD-16), compared with run 1's table | (a) Exit 2; exactly `gap_has_end`, `streams_closed`, `history_complete` (the mid-run drop), `wire_keeps_up`, `threads_match`, `subagents_present` and `server_counts_match` fail; `provider_errors.wire` reads the planted count once. (b) Exit 0 or 1, never 2; `history_complete` `pass` or `named`; no 10-minute bucket with harness calls and 0 wire calls unless covered by a named GAP; `provider_errors.wire` equals, within one per call, the failed calls in Codex's own rollouts (U1 native errors, listed apart) |
| R2 | **A clean run passes.** Claude Code, `direct`, `full` (PRD-12) | Exit 0, or exit 1 where the only `named` result is an `after_stop` GAP. Every other check `pass` or `skip`. `history_complete` is `pass` with no `history_text_unmatched` warn, and checks the main thread and every subagent. `threads_match` covers the main thread and every subagent. `server_equals_local` passes. `secrets` finds 0 known values |
| R3 | **A killed recorder is named, not failed.** Codex kill test (PRD-01 acceptance 3, PRD-02 R1) | Exit 1. Two `recorder_down` gaps. Every harness call missing from the wire falls inside them, so `wire_keeps_up` and `threads_match` are `named`, and `history_complete` is `pass` or `named` by them. No `fail`. The same holds for PRD-02's crash-loop and `goalcup recover` cases: one `null_mode` or `watchdog_lost` gap, `named` |
| R4 | **The dodger case is honest.** Codex with its model host marked `pinned` on the pad | Exit 1, never 0 or 2. Every connection to that host is a tunnel with GAP `pinned`; `listed_hosts_decrypted` is `named`; `wire_keeps_up` is `named` by them; `history_complete` is `skip` "nothing decrypted to the model host"; the label is `native_only` |
| R5 | **Hermes passes on the history check; its log check runs per call or by totals.** Hermes, DeepSeek key, one delegated child (PRD-12 core row) | `history_complete` passes (Hermes resends the conversation to DeepSeek); this is the proof. The optional log check: `harness_log` is `per_call` or `totals`, as PRD-09 reads it. With `per_call`, `wire_keeps_up` and `threads_match` pass as for Codex. With `totals`, `wire_keeps_up` is `skip` "no per-call times" and `threads_match` passes in the totals form (per session, or per entry when wire calls carry no session id). Either way `subagents_present` finds the child from `parent_session_id` |
| R6 | **Load.** PRD-16 A1 (Codex, 2 hours) and A2 (Claude Code, up to 20 subagents, 60 minutes, one recorder kill), as PRD-16 runs them under the 10 USD cap (PRD-03 acceptance 10) | `history_complete` and `threads_match` pass or are `named` for every thread (A2's kill window is named by its `recorder_down` gap); `provider_errors` shows any 429s. Time for `check` is logged as a measure, never a pass line (PRD-00: no speed number fails a gate) |
| R7 | **Secrets.** The pad's planted `.env` and prompt (PRD-05 A1), all three cases | 0 known values in local part files and server copy, the run CA key included (R3b); every planted shape has a MASK |

The run 1 converter is not built, so run 1's own server copy is not re-checked (PRD-00 "Not in this
build"). Run 1's numbers stay in the run report.

### Automated tests that guard them

- `check_invariants.rs`: a valid entry built with `PartWriter` exits 0; one mutation per invariant
  (1 to 14), a torn tail, a repeated native `path` and `gen` in one boot, and an unknown record kind
  give the right id and exit; GAP reason 8 gives `warn`, not a crash.
- `check_run1_shape.rs`: a small record shaped like run 1 (root and four Codex-shaped threads, wire
  stops at minute 49 while rollouts grow to minute 120, six rate-limit errors sent both as an `error`
  event and a `response.failed`, three GAPs with start equal to end and unknown loss, two streams with
  no END in a sealed entry, one child with neither leg, and one root call at minute 20 dropped with
  no GAP while later requests resend its tool-call answer). Pass: exactly those checks fail, exit 2,
  `provider_errors` reads 6. The same record with the loss removed exits 0, so the checks do not fire
  on a clean run. It runs against PRD-09's real U1 sink once that lands (a stub before).
- `check_named.rs`: the clean record with one `recorder_down` gap that covers the missing calls: exit 1,
  results `named`. Also: a listed tunnel with each GAP 6, 7, 13, 14, and a `hello_too_big` tunnel with
  GAP 1, is `named`; with no GAP, `fail`; an `after_stop` GAP is `named`; a Hermes-shaped totals record
  passes `threads_match`; a `tls_refused` GAP with `{"alert": false}` raises no `refusal_after_trust`;
  a `recorder_down` gap with cause `null_mode` over 10 minutes and one with cause `watchdog_lost`
  over 2 hours are each `named`, never `fail`; with `--live` and a fake `status.json` `mode: "null"`,
  a stale COUNT is `named`, and without it `fail`.
- `check_history.rs`: one small record per request shape (Chat, Anthropic Messages, Responses with
  resent `input`, Gemini). A clean chain passes. One call dropped with no GAP while a later request
  resends its tool-call answer: `fail` naming the thread and `sid`; the same drop inside a
  `recorder_down` gap: `named`. A text-only answer missing: `warn` `history_text_unmatched`. A chain
  whose first request holds an unknown answer: `warn` `history_unanchored`. Requests with only
  `previous_response_id`: `skip` "history held by the provider". A request with the same thread id
  whose history no longer extends its chain: `skip` "compaction" from there on, earlier turns still
  checked. An answer copied from another thread matches. No network call is made (the test runs with
  no network).
- `check_conn_bytes.rs`: a decrypted connection whose second request is missing with a `conn` GAP is
  `named`; the same with no `conn` GAP is `fail`; normal framing overhead stays `pass`.
- `server_compare.rs`: a fetched copy equal to the local part files passes; one flipped byte, a missing
  middle chunk, a wrong `model_requests` and a wrong `counts.off_ms` in `fetch.json` each fail; a
  fetched copy with `final` null reports an open end (exit 1) and `server_counts_match` `skip`.
- `secrets_scan.rs`: a known value split across two DATA records, in base64 at each alignment,
  percent-encoded and spread over three SSE deltas is found; the same value as `*` is not; a glued
  shape gives `warn` `glued_shape`; the same shape after a space and unmasked gives `fail`; a 20 KiB
  `fail_closed` span is reported, a 20 KiB span of another rule fails; a known value in a plain log
  file and on stdin is found.

## Risks and blind spots

- **False alarms in `wire_keeps_up` and `threads_match`.** A harness may write a call twice, or its
  log format may change between versions (Claude Code says it does,
  `docs/plans/2026-10-30-audit/D-harnesses.md:276-278`). Guard: the rule only fires on harness above
  wire; the tolerance is max(3, 2%); R2 and R6 must pass on real runs before the pad trusts it.
- **Only Codex, Claude Code and Hermes have readers in 1.0.** Hermes is per call when `state.db` rows
  carry usable times, else the totals form. Every other harness gets `skip` on the optional leg
  checks until a pad sample adds a reader. Then a silent loss is caught by the history check, the
  integrity checks, `conn_bytes` and `recorder_alive`, and the text output says the log was not
  read. The totals form cannot say when a loss happened, only that one did.
- **The history check cannot see a wire that stops for good,** nor the last call of a thread: no
  later request resends those answers. `recorder_alive`, `streams_closed` and, where session files
  exist, `wire_keeps_up` cover that. It is also blind where the provider holds the history
  (previous-response id) and after compaction, and says `skip` there.
- **The history check may misread a harness that rewrites answers.** A harness that trims answer
  text, drops tool calls it judged failed, or injects its own assistant turns would look like a
  missed call. Guard: only a missing tool-call id can `fail`; text-only misses are `warn` until R2
  and R6 show the text rule holds; a chain's first request is never `fail`.
- **Hermes wire calls may carry no session id.** Then the totals form compares per entry, and a lost
  child is visible only if the sum falls short.
- **A loss before a stream opens is hard to see.** If the proxy drops a whole connection without a
  `conn` record, only the harness's log or the history check can show it. For a harness with no
  session files that does not resend history, nothing can.
- **`conn_bytes` slack is a guess.** Head sizes are approximate and HTTP/2 framing is not in the
  stream bytes. The `fail` line (50% and 1 MiB) is loose on purpose; the `warn` line is tight. Three
  pad runs measure the real overhead before PRD-16 tightens either.
- **`refusal_after_trust` is a hint.** Two tools on one runtime share a fingerprint, so a tool's
  refusal can look like the harness's. It is `warn` only.
- **Thread hints are per harness.** A harness whose wire requests carry no thread id gets run-level
  `wire_keeps_up` only; `threads_match` is `skip` or the totals form.
- **The verifier trusts the record it reads.** Signing is cut, so it proves the server copy whole and
  consistent, not that the participant's machine was honest. The server's `received_at` bounds time.
- **`shapes.tsv` lives in another crate.** A path change breaks this crate's build at once (loud, not
  silent). A test here parses the file.
- **Cost on a busy week.** About 140 GB raw (PRD-01 section 14) is one decode and one SHA-256 pass,
  estimated 10 to 30 minutes on 4 vCPU (unverified). `check --live` reads the whole entry each time.
  That is fine for 2 hours; for a week the watchers run it hourly.
- **Judges lose the decoded research report and the judge page until after the merge.** PRD-09's usage
  JSON, S6 `label` and the coverage line stand in.

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Harness above wire tolerance | max(3 calls, 2%), with a 2-minute lag | PRD-16 after R2 and R6 |
| History check, text-only answer missing | `warn` `history_text_unmatched` (a missing tool-call id is `fail`) | PRD-16 after R2 and R6 |
| Wire ends before native | 10 minutes | PRD-16 |
| COUNT silence | 180 s, after sleep | PRD-01 and PRD-02 if COUNT cadence changes |
| `conn_bytes` | `warn` at 10% and 1 MiB; `fail` at 50% and 1 MiB with no `conn` GAP | PRD-16 after three pad runs |
| `mask_failclosed` | `warn` above 1 MiB in one stream | PRD-05 |
| Biggest MASK span (other than `fail_closed`) | 16 KiB | PRD-05 |
| Secret scan join overlap | 4 KiB | PRD-05 if a rule matches longer values |
| Timeline bucket | 10 minutes | anyone |
| `check --live` cadence | every 5 minutes; hourly past 24 hours | PRD-16 |

## Builder notes

Order of work (starts at H+0 from fixtures, review B O5):

1. `main.rs`, `input.rs` and `integrity.rs` over PRD-01's reader, with `check_invariants.rs`. Publish
   S6 with real `inv*` results and `skip` for the rest by H+3: the core slice and the phase 1a
   watchers need exactly this (PRD-00 core slice: invariants, gaps have an end, counts recount).
2. `report.rs` counts, `bytes`, `counts_match_seal`, the label.
3. `honesty.rs` (`listed_hosts_decrypted`, `conn_bytes`, `mask_failclosed`).
4. `legs.rs` against a stub `CallTable`; switch to PRD-09's sink when U1 lands (H+4). Build
   `check_run1_shape.rs` with Codex-shaped bodies so PRD-09's real reader runs on it. The totals form
   when PRD-09's Hermes reader exists (second slice). Then `history.rs` with `check_history.rs`, on
   U1's pairing (stub before it lands). It needs no session files, so it is the C1 proof for every
   harness that resends history.
5. `server.rs` as soon as PRD-07 publishes S13 (`fetch.json`); then `secrets.rs` (second slice, with
   S12 by H+3).

Traps from the audit:

- Do not compare raw wire attempts with harness calls: the wire counts retries and warm-ups (appendix
  M section 5.6 lesson 1). Compare non-error wire calls, and only flag harness above wire.
- Do not count an `error` event and a `response.failed` for one call twice (`openai.rs:475-486,
  545-558`); U1 gives one row per call, use its outcome.
- Take errors from the wire leg always; today's verifier took them from the token leg
  (`stats.rs:949-956`), which hid run 1's 429s.
- Never turn a cap into a verdict (`reader.rs:1694`): a large run is not an invalid run.
- Never decode a chunk alone: a chunk continues its part file's frame (PRD-01 section 12).
  Concatenate, then decode.
- Never print a byte of a DATA record in `check` or `secrets`; only `dump` prints payload, and only
  on request.
- The `include_str!` path is `../../goalcup/src/mask/shapes.tsv` relative to `src/secrets.rs`; it ties
  the two crates to one workspace, which is fine (neither is published to crates.io).
- `rusqlite` with `bundled` compiles C; the first build in a build box is slow. Declare it in phase 0
  so the cache is warm.

How a watcher proves it in a real run, all in a box (section 1 "Where it runs"): after each pad run,
`goalcup-verify check --json` on the entry and `goalcup-verify check --server <fetched> --local <entry>
--known <keys>` in the run's pad box or a fresh check box; the pad table reads `result` and the mapped
checks. During a long run, `check --live` every 5 minutes in the pad box via `sandbox exec` under
`nice -n 19`: a `fail` on `history_complete`, `wire_keeps_up` or `recorder_alive` is the alarm run 1
never raised.

Traps for the history check: never call a provider to look up usage or history (ledger T4); match
against every response in the input, not only the same thread; keep only new turns per chain, never
whole histories; never print turn text, only hash prefixes and positions.

## Asks still open

All asks are ruled by PRD-00 R3-6 and R3-11 and are in PRD-07, PRD-12, PRD-14 and PRD-15. Open:
PRD-09 adds `native_form` and per-session `api_call_count` to U1 (G3-X7).

## Review answers

Sources: CC = `prd-consistency.md` item; A = review A finding; B = review B item.

| Item | Verdict and what changed |
|---|---|
| CC 1, 7, B C25 (asks absorbed, seam numbers) | Accepted. S12 (shapes), S13 (fetched layout) and U1 are PRD-00's numbers; old S9 and S10 names removed. Remaining asks listed in "Asks still open" |
| CC 17, B A11 (dodger exit code) | Accepted with PRD-12's line: exit 1, never 0 or 2 (R4). Exit 1 comes from the named `pinned` tunnels, so it holds even where `wire_keeps_up` is `skip` |
| CC 28, B C2 (reasons 8 and 9, reason 12 twice) | Accepted. Reserved 8 and 9, and any unknown number, show as `other(n)` with `warn`; reasons 1 to 15 as PRD-01 revision 2 fixes them |
| CC 29 (`meta.from`) | Accepted. inv4 counts from `from`; inv14 checks resume across boots |
| CC 33 (`plain_up`, `plain_down`) | Accepted. `conn_bytes` uses them and END `wire` |
| CC 39, B C3 (invariants) | Accepted. Integrity now runs PRD-01's 14 invariants; the old honesty checks `streams_closed`, `seen_matches_written`, `gap_has_end`, `recorder_alive` are invariants 10 to 13 under the same ids, so PRD-12 and PRD-16 need no change |
| CC 40, B N1 (run 1 converter) | Accepted: cut. R1 now proves run 1's shape on the synthetic record and on A1; acceptance and interfaces no longer name a converter |
| CC 48, B N5 (silent-drop hook, R5) | Accepted: cut. R5 replaced by the Hermes totals proof; no `test-hooks` feature in this crate |
| CC 52 (named tunnel reasons) | Accepted. GAP 6, 7, 11, 13, 14 are `named` in `listed_hosts_decrypted` and `wire_keeps_up`; `sign_in` is `pass`; a listed tunnel with no GAP is `fail` |
| CC 65, B C26 (rules as data) | Accepted. `crates/goalcup/src/mask/shapes.tsv` read with `include_str!`, compiled without the guard; only `fail_closed` spans may pass 16 KiB (PRD-05) |
| CC 79, B C9 (fetched layout) | Accepted PRD-00's S13: `rec.gcr.zst` and `fetch.json` with the entry row and `recording`. B C9's `server.json` name rejected (it clashes with the local per-run file). PRD-07 still writes `server.json` in revision 1; ask 2 |
| CC 83, B M4 (dependencies) | Accepted. `zstd` and `rusqlite` (bundled) declared in phase 0; checked against PRD-09's interfaces row |
| CC 84 (rate-limit range) | Accepted. 95 to 99 everywhere; checked the run report: about 97 error events, 99 `response.failed`, 95 failed calls in Codex's view. The S6 example is marked as an illustration |
| CC 85 (U1 enum) | Accepted. PRD-09's U1 used as is (`RateLimited`, `Cancelled`, `whole`); the old sketch removed; one-per-call folding is U1's |
| CC 86, 95 (`test-hooks` guard, hook registry) | Accepted. This crate declares no `test-hooks` feature; PRD-14 drops the silent-drop row (ask 4) |
| CC 105 (`failed` only in null mode) | Accepted in `server_counts_match`: it recomputes PRD-06's `recording` rule from the seal totals instead of its own rule |
| A 1 (refusal learning tunnels the harness) | Partly. The fix is PRD-03's; this PRD adds a `refusal_after_trust` `warn` so a watcher sees it |
| A 2 (parser loss on a reused connection) | Accepted part (b). `conn_bytes` now fails above 50% and 1 MiB unaccounted with no `conn` GAP; `warn` stays at 10%. New `check_conn_bytes.rs` |
| A 3 (fail-closed masking) | Accepted. `mask_failclosed` `warn` and `mask_failclosed_bytes` in counts; `fail_closed` spans exempt from the 16 KiB rule |
| A 6 (unlisted model host, live signal) | Routed. The live line in the participant's run is PRD-10's; watchers already get it from `check --live`. `long_tunnel` notes stay a `warn` fact here |
| A 8 (processes after the seal) | Accepted the reading part: GAP 15 `after_stop` is a named loss, never off time |
| A 9 (SQLite copies only at the seal) | Accepted the reading part: periodic copies are read by `set`; `subagents_present` and the manifest check accept `snapshot` streams without comparing sizes |
| A coverage gaps | Routed to PRD-12 and PRD-16; nothing here needs a hook |
| B A4 (core slice) | Accepted. Builder step 1 is exactly the core slice's `check` |
| B A7 (unreadable database copy) | Accepted. `skip` with the reason; the pad maps it to INCONCLUSIVE |
| B A8 (only two per-call readers) | Accepted. Section 3.1 says it plainly |
| B R1 (Hermes cannot pass C1) | Accepted option (a), PRD-00's default: the totals form in `threads_match` (section 3.1, R5). Needs PRD-09 ask 1 |
| B N2, T9 (judge view and its golden) | Accepted: after the merge. Partly kept: S6 `label` and the coverage line (about 40 lines), because RULES and PRD-13 cite the label |
| B O5 (start at H+0) | Accepted. Builder notes |
| B T3 (speed numbers in gates) | Accepted. R6 logs `check` time; it is not a pass line |
| B W10 (kept parts) | No change needed: `server_equals_local` already needs `keep: true`, and a pruned copy is `skip` that PRD-12 turns into FAIL |
| PRD-13 line 203 (size numbers) | Accepted. S6 `bytes` gives raw, compressed and framing bytes |
| Own check | Today table re-checked in code on 30 Oct: exit mapping, caps (`:78` is the goal-stream cap, `:1384` its use), hole rule, double count, error source, `receiver_thread_ids`; `create_output_file` starts at `:262`. Size estimate restated at about 1,400 src lines |

## Alignment answers

Fix lists g1, g2 and g3 for this PRD, checked against the ledger, PRD-01, PRD-03, PRD-07, PRD-09,
PRD-10, PRD-16 and the old verifier code.

| Fix id | What was done |
|---|---|
| L2-04 | Applied. `counts` row: a boot's total is its last COUNT; for a dead boot, the COUNT in its recovery frame (PRD-01 section 9, which already says so) |
| G2-11 | Applied. R6 now points to PRD-16 A1 (Codex, 2 hours) and A2 (Claude Code, up to 20 subagents, one kill) under the 10 USD cap, not "A2 Codex" |
| P03-4 | Applied. `hello_too_big` tunnels with GAP 1 `memory_guard` count as `named` in `listed_hosts_decrypted`, `wire_keeps_up`, the PRD-03 interfaces row and `check_named.rs`. Also fixed the old list "6, 7, 11, 13, 14": PRD-01 says 11 is never a tunnel reason, so tunnels are 1, 6, 7, 13, 14 and 11 is a `conn` GAP after a parser fault, still `named`. The CC 52 review row stays as history |
| P03-22 | Applied. `refusal_after_trust` skips a `tls_refused` GAP with `{"alert": false}`; a case in `check_named.rs` |
| X2-11 | Partly. This fixer owns only this file, so `REVIEW-HISTORY.md` is not created here and "Review answers" stays in place for whoever does the move. Every binding ruling is already in the body; "Asks still open" is cut to the one open ask (G3-08b) |
| G3-X1 | Applied. Section 1 "Where it runs" (ledger B1): every `goalcup-verify` command runs in a box, `check --live` under `nice -n 19` via `sandbox exec`, the known-values file holds only that box's values; the conductor runs only `fetch-run`, sandbox control and git. Owner decisions bullet, PRD-12 and PRD-16 interface rows and the watcher paragraph follow |
| G3-X21 | Applied. A glued shape hit (previous byte a letter, digit or `_`, PRD-05 section 4) is `warn` `glued_shape`; any other shape hit is `fail` (R3-11 drops "unless the pad lists it"). R7 drops "glued keys are reported"; `secrets_scan.rs` case updated |
| G3-X7 | Applied. Section 3.1 says where `harness_log` comes from, Hermes is `per_call` when its rows carry usable times, else `totals`; per-entry totals form as the fix wrote it; `native_only` and `native_until` rules. PRD-09 interfaces row, R5 and Risks updated |
| G3-X24 | Applied. `server_chunks_whole` with `final` null checks continuity and reports an open end (exit 1); `server_counts_match` is `skip` "not sealed"; a `server_compare.rs` case |
| G3-08a | Applied. `secrets <path>...` scans raw files and stdin with the same matcher; a test case; secrets estimate 200 to 230 lines |
| G3-08b | Applied. "Asks still open" cut to the PRD-09 ask; `server_chunks_whole` hashes every slice by `bytes`; PRD-07 row lists `bytes` and the last heartbeat's `counts` |
| G3-08c | Applied. `server_counts_match` also compares `fetch.json` `counts` with the seal totals key by key; a wrong `counts.off_ms` case |
| G3-08d | Applied. Each owner-decision bullet starts with its ledger id; the T4 bullet names the four checks and gives usage stats to PRD-09 (PRD-12 C5) |
| G3-08e | Applied. R2 is Claude Code `direct` `full`, exit 0 or exit 1 from an `after_stop` GAP only, every subagent covered |
| G3-08f | Applied. R1(b) compares `provider_errors.wire` with Codex's own failed calls, within one per call |
| G3-08g | Applied. `wire_keeps_up` states the cover window and that a GAP failing `gap_has_end` covers nothing |
| G3-08h | Applied. Coverage line ends "Recording off: 0 times, 0 s." from `off_ms` |
| G3-08i | Applied. Checked in code: `lib.rs:34-35` and `:39-53`, `create_output_file` at `goalcup-verify.rs:264-294` (section 1 and section 6) |
| G3-X36 | Applied. PRD-10 row: the check line goes to `goalcup.log` only |
| G3-X33 | Applied. PRD-17 row: only H10 and K15 write a part file and run `check` on it, asserting exit 0 |

R3a, R3b, T4 carried in. T4: new check `history_complete` (section 3.2, `src/history.rs`,
`check_history.rs`), offline on the record: every earlier assistant answer resent in a later request
must exist as a recorded response; a missing tool-call answer is a finding with its thread (`fail`,
or `named` under a gap), a text-only one `warn`; `skip`, said, for previous-response-id requests and
after compaction. `wire_keeps_up`, `threads_match` and the `--harness-files` manifest are now an
optional extra that runs only where the harness keeps session files; their `skip` is never a
failure, and PRD-12 C1 rests on `history_complete`. "No outside API" stated in Goal and Owner
decisions. R1 to R6, S6 (`history` keys), coverage line, Risks, defaults, builder notes and the
PRD-09, PRD-12, PRD-14 (seven test files) and PRD-16 rows follow. R3a and R3b: `recorder_down` gaps
with cause `null_mode` or `watchdog_lost` are named of any length (inv6); `--live` `recorder_alive`
is `named` in null mode, `fail` otherwise (the `goalcup recover` alarm); the run CA key from
`rec/run-ca.key` joins the `--known` file and R7; a `check_named.rs` case. Estimate up to about 1,600
src lines.

## Amendments

### 2026-10-30, replan after PRD-17 phase 0 slice (merge 1423f8f9)

- **Four cases the lessons gate waits on** (lessons log 10:45Z; `LESSONS.md` marks them `(name
  pending)`). Name each in the night log when written; the lessons lane puts the name in
  `LESSONS.md`. Each is a point 1, 3 or 4 case, so ledger B10 keeps it:
  - PRD-17 FW3: a part folder whose first parts are gone exits 1 (only the server copy can be judged
    whole), next to the `server_compare.rs` missing-middle case.
  - PRD-17 VR2: a panic inside a check prints file and line, never record text (PRD-09 has the
    `usage` half in `usage_run1.rs`).
  - PRD-17 VR3 (`check_invariants.rs`): every mutation gives its id and exit 1 or 2, never 3.
  - PRD-17 VR4 (`check_named.rs`): a clean record with an `interrupted` END exits 0.

### 2026-10-30, replan after PRD-12 slice 2 (merge c7649890)

`scripts/pad/check.py` is on integration, built to this PRD's published shapes. Four things it reads
are not fixed here yet (pad lane log 11:44Z and 12:02Z). Post each answer as a seam line in the night
log; the pad lane follows.

- **`secrets --json` output.** C3 needs MASK spans counted by rule kind under a `masks` key, a map
  from the S12 kind name (`openai_key`, `github_token`, `google_key`, `generic_secret_assignment`,
  `jwt`, `private_key`, `url_password`, ...) to a span count. The Secrets paragraph already says "Spans are counted
  by rule"; the key name is what is missing. Until it exists, every pad row is C3 INCONCLUSIVE "no MASK
  counts by rule". `check.py` reads hit counts under `hits`, `known_hits` or `shape_hits` and falls
  back to the exit code (0 and 1 no hit, 2 a hit, anything else no scan); name the one key.
- **A `recorder_down` gap's cause and length in `check --json`.** The kill rows (PRD-12 section 3.1)
  need each gap's cause (`recorder_down`, `null_mode`, `watchdog_lost`), start, end and length in ms.
  `check.py` reads `facts.gaps` as a list of `{cause, start, end, ms}` when present, else `counts.gaps`
  keys; publish which.
- **`keep` from the start event.** C4 needs to know the parts were kept (`GOALCUP_KEEP_PARTS`, PRD-01
  start event `keep`). S6 does not expose it; `check.py` reads it from the run's settings and says so
  in the C4 line. A `keep` field in the S6 entry would let C4 read it from the record.
- **The interfaces row for PRD-12 contradicts section 3.1.** The row says C1 is INCONCLUSIVE when
  `history_complete` and both optional checks are `skip`; section 3.1 says a `skip` never makes C1
  INCONCLUSIVE on its own, and PRD-12 section 8 (K11 (c), ledger T4: the history check is "skipped
  where it does not" resend) lets a history `skip` pass C1. The pad follows PRD-12; read the row's
  clause as superseded. Nothing in this crate depends on it: the C mapping lives in `check.py`.

### 2026-10-30, replan after PRD-05 phase 0 slice (merge 82484521)

- **S12 is on integration** at `crates/goalcup/src/mask/shapes.tsv`, in the shape PRD-08 reads:
  `#` lines are comments, blank lines are skipped, data lines are `kind`, tab, `guard`, tab, regex,
  20 data lines, no tab inside a regex. Two rules from the file's header bind the reader: the span is
  group 1 when the regex has a capture group (`url_password`), else the whole match; and
  `private_key` uses `(?-u:[\x00-\xFF])`, which only `regex::bytes` accepts, so the scan compiles
  with `regex::bytes`. PRD-05's box check found the 19 old regexes equal to `redact.rs`.
- **Entries built on the S2 stub** (PRD-05 review A1). Until PRD-05's core slice merges, every MASK is
  `fail_closed`, `start.mask_rules` is `["fail_closed"]`, `mask_meta` stars every meta string
  (header names, method, host, native `path`) and no `meta.masked` is written. A check that needs
  `meta.masked` or a native `path` fails on such an entry; the failure is the stub, not a recorder
  defect. Check-test fixtures are hand-written records or built with `Masker::passthrough_for_tests()`,
  never with the stub.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 1 (merge d10feef7)

- **Size decision.** `wc -l` at d10feef7: src 1,635 (`integrity.rs` 1,089, `main.rs` 209,
  `input.rs` 192, `report.rs` 122, `usage/mod.rs` 12 (PRD-09's placeholder), `lib.rs` 11), 117%
  of the 1,400 row with four of the ten source files written; tests 1,471 (`tests/common/mod.rs` 907,
  `check_invariants.rs` 346, `check_named.rs` 218), 134% of the 1,100 row with three of the seven
  files. This PRD estimated 630 for the files written (input 150, integrity 380, report and main 100);
  they came to 1,623. The fixer's line (judge 13:09Z) names points 5 and 6: all 19 COUNT counters plus
  `hosts` and `tunnel_hosts` are recounted to PRD-02's `pipeline/counts.rs` rules, and the seal
  totals per recovery frame, so the counts the participant saw are proven. **Decision:** the crate's
  ceiling is 2,400 src and 1,700 tests by `wc -l` at the end of the last PRD-08 slice (`usage/` is
  PRD-09's row once PRD-09 replaces the placeholder). The extra lines serve points 5 and 6 as above and
  point 3 (every `recorder_down` cause, null mode and recovery frame is `named`, never a crash or a
  false `fail`). Per-file ceilings: `integrity.rs` 1,000, `input.rs` 150, `main.rs` 200,
  `report.rs` 150, `lib.rs` 11, `honesty.rs` 150, `legs.rs` 260, `history.rs` 170, `server.rs`
  130, `secrets.rs` 175; tests `tests/common/` 900 including the run 1 shape builder,
  `check_invariants.rs` 330, `check_named.rs` 220, `check_run1_shape.rs` 80, `check_history.rs`
  80, `server_*.rs` 60, `secrets_*.rs` 30. Cut first: B13 (below, about 20 test lines); the PRD-01
  ask (below, about 40 lines of `input.rs`); `integrity.rs` down to 1,000 in the slice that next
  touches it by folding the per-check call sites rustfmt spread (judge 13:09Z) into `Pass::set` and
  `Pass::fail`, never by dropping a check. Reviewer B checks each merge against these numbers; past
  them needs another decision line. The Rust test total stays at most about 14,200 (PRD-14 section 3,
  raised only by the owner): PRD-16's end-of-build report nets this area's extra against the others,
  and only a total past 14,200 becomes an owner question.
- **S6 as merged** (answers to PRD-12's asks; judge 12:19Z, 12:35Z, 13:06Z). The `inv6` check's
  `facts.gaps` is a list of `{cause, start, end, ms}`, one per `recorder_down` gap, `start` and
  `end` as UTC text. Each entry carries `keep` from the start event. `counts.gaps` maps GAP reason
  name to count; `gaps_named` facts are keyed by reason name with `{count, lost_bytes,
  lost_unknown}`; `streams_closed` facts `open` lists `"boot.sid"`; `counts_match_seal` is
  `skip` when unsealed or pruned. Every later check id is already in the report as `skip` "not built
  in this slice" (seven local ids, plus the three `server_*` ids under `--server`); `history`,
  `harness_log` and `provider_errors` are `null` and `threads` is `[]` until legs and history land.
  `secrets --json` and its `masks` key still come with the secrets slice.
- **Options as merged:** `check <path> --json --live --server --local <entry dir> --known <file>
  --harness-files <manifest>`; the last three are accepted and read nothing until their slices.
  `--live` reads null mode from `status.json` `off.since` only; a live entry with nothing durable
  yet is `inv1` `skip` and exit 1; a part file that cannot be opened is exit 3.
- **Pruned entry (L-FW3, local half).** Built: a folder whose first parts are gone exits 1 with
  `parts_pruned` named (`inv3`, `inv7` and `inv9` do not fail in the first boot read;
  `counts_match_seal` is `skip`). No test names it yet. The next slice adds one to `check_named.rs`
  and posts its name in the night log; the server half stays in `server_compare.rs`.
- **Panic output (L-VR2).** `main.rs` installs a panic hook that prints file and line only and exits 3.
  No test yet; it stays with the server slice as planned. This crate has no test hook (ledger L6), so
  the case needs an input that makes a check panic; if the builder finds none, it says so in the night
  log and the lessons lane records the case as a review check of the hook.
- **PRD-17 names** (judge 13:06Z): L-VR3 is `check_invariants.rs`
  `vr3_every_mutation_fails_its_check_with_exit_2` (a 21-row table; each row asserts its own check
  fails and exit 2, stricter than "1 or 2, never 3"); L-VR4 is `check_named.rs`
  `clean_record_with_interrupted_end_exits_0`. The 12:49Z name `every_mutation_exits_1_or_2_never_3`
  is superseded.
- **The COUNT recount and PRD-02's `pipeline/counts.rs` move together.** `inv7` and
  `counts_match_seal` compare every key the recount knows and ignore gauges and unknown keys; a COUNT
  or seal totals that leaves a known key out fails. A change to a counting rule in either file is a seam
  line and the same rule in the other file in one merge window (PRD-02 note).
- **Ask of PRD-01 (optional):** let `read::Reader` or its `Tail` report the raw bytes decoded and
  whether input ended inside an open frame, so `input.rs` drops its `Raw` and `Meter` copy (about
  40 lines). Until then the copy stays.
- **Open nit B13** (`tests/common/mod.rs`, `Entry::recovered_before_first_link`: the local `write`
  closure copies `Entry::put`, `at` is split into `at` and `bare` only to skip tallying a dead
  `start`, and two `Record::Event` literals are hand-written; reviewer B's shorter form keeps `put`
  the only tally-and-write path, about 20 lines) goes to this PRD's next slice. Acceptance lists it as
  known if it is still open at the end.
- **The interfaces row clause on C1** is still the stale one (PRD-12 note); the pad follows PRD-12.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-09 slice 1 (merge f5a529c8)

- **U1 is on integration as a compiling stub with its final names** (PRD-09 note). `legs.rs` and
  `history.rs` build against it now; the stub's table is `CallTable::default()`: no rows, not
  readable, `Reason::NoReader`, `children: None`. Read as merged:
  - `usage::Calls::new()` takes no argument (the interfaces row's `Calls::new(&Start)` is stale;
    PRD-09's U1 wins, item 85). Push every record in order, then `finish()`.
  - Build `CallRow`, `Thread` and `CallTable` only with `CallRow::new(leg, entry, boot, kind)`,
    `Thread::new(id)` and `CallTable::default()` plus field sets or struct update, including in
    `tests/common/` (the run 1 shape builder); never a full struct literal.
  - `children: None` means parentage was not read: `subagents_present` (C2) is `skip` on it, never
    `pass`; on `Some` it runs on what was read. This replaces the judge-usage 14:36Z rule keyed on
    `NoReader`.
  - `usage_from` is `Option<Leg>`; `Outcome::is_error()` is PRD-08's "error" as before.
- **`readable` becomes a method in PRD-09's core slice** (nit B10, PRD-09 note): read
  `table.readable()` and `thread.readable()` once that slice is on integration; the `harness_log`
  rule (`readable` false gives `none`) is unchanged. If `legs.rs` or `history.rs` merges first and
  reads the field, the field stays and PRD-09 drops B10; PRD-08 changes nothing either way.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-15 slice 6 (merge 9e89a22d)

- `docs/RECORD-FORMAT.md` now documents the `check --json` report from `report.rs` as merged: the
  top-level and entry keys, the check ids with their `inv` numbers, the exit codes, and the ten ids
  that are always `skip` "not built in this slice" (plus `server`, `harness_log`, `provider_errors`,
  `history` and `threads` not filled). A later PRD-08 slice that adds or renames a key or check id,
  changes an exit rule, or builds a skipped check names each change in its merge note, so PRD-15's
  next slice updates the page (PRD-15 note). No code change is asked of PRD-08.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 2 (merge 669f5d92)

- **Built and checked.** `check --server <fetched> [--local <entry dir>]` gives real
  `server_chunks_whole`, `server_equals_local` and `server_counts_match` results and the S6 top-level
  `server` object `{chunks, whole, equals_local, db_match}` from PRD-07's S13 layout. Gate on
  669f5d92: `gate fast: PASS in 89s`, nextest 136 of 136 (goalcup-verify 25 of 25). Nine paths
  changed, all owned here. The slice 1 debts are paid except `main.rs`: `integrity.rs` 982 of
  1,000, B13 done in `tests/common/` (877 of 900), `check_invariants.rs` 329 of 330.
- **Rules as merged (S6; PRD-12, PRD-15 and PRD-07 notes).** Under `--server`, a copy with no
  `fetch.json` gives all three server checks `fail`; a bare `rec.gcr.zst` checked without
  `--server` still skips them. `fetch.json` `missing.why` set gives `server_chunks_whole` `fail` with
  `first.o` = `missing.offset`, also when `final` is null. `final` null alone gives
  `server_chunks_whole` `named` (open end, exit 1, never `fail`) and `server_counts_match` `skip`
  "not sealed", as PRD-07 asked. A seal with no `reason` or no `totals` is reported, never exit 3.
  Record sums and offsets saturate. `server_equals_local` is `skip` when the `--local` entry's first
  part is pruned or its `rec/` is empty; a `--local` folder that never held a record is exit 3.
  Top-level `equals_local` and `db_match` are true only when every entry passes, false on any `fail`,
  else null; `whole` is true only when every entry passes.
- **Lesson names** (judge 14:56Z, 15:13Z, PRD-17 note): L-FW3 (both halves, plus the wrong-path case)
  is `tests/server_compare.rs` `fw3_pruned_entry_is_named_and_server_copy_stands_in`; the
  `check_named.rs` test the slice 1 note planned for the local half is not needed. L-FW9 is
  `server_compare.rs` `fw9_one_flipped_byte_fails_the_copy`. **L-VR2 is settled** by
  `server_compare.rs` `vr2_seal_without_reason_or_totals_is_reported` and the `("inv5",
  m_mask_at_top)` row of `check_invariants.rs` `vr3_every_mutation_fails_its_check_with_exit_2`;
  both exited 3 before the fix. The panic hook in `main.rs` stays as a review check for inputs no
  test reaches.
- **Size decision** (`wc -l` at 669f5d92). Src without PRD-09's `usage/` is 1,724: `integrity.rs` 982,
  `server.rs` 201, `main.rs` 206, `input.rs` 198, `report.rs` 126, `lib.rs` 11. That is six of ten
  files, 72% of the 2,400 crate ceiling and 123% of the PRD-00 row (1,400). Past their per-file
  ceilings: `server.rs` (201 of 130), `input.rs` (198 of 150, the PRD-01 ask is still open) and
  `main.rs` (206 of 200, review nits A9 and B3 still open). Tests are 1,612: `tests/common/` 877,
  `check_invariants.rs` 329, `check_named.rs` 218 and `server_compare.rs` 188 (60 was set). With the
  four unwritten files at their ceilings (honesty 150, legs 260, history 170, secrets 175) and
  `report.rs` at 150, src comes to about 2,503. With `check_run1_shape.rs`, `check_history.rs` and
  `secrets_*.rs` at theirs, tests come to about 1,802. **Decision:** `server.rs` is capped at 201 (as
  merged) and `server_*.rs` at 190. The extra lines serve point 5: the server copy is whole, equal
  to the local bytes and matches its row, checked by entry id. They also serve point 3: review fixes
  A1 to A4, F2 and F4 replaced exit-3 crashes and false passes on odd seals and odd `fetch.json` with
  named results. The tests are lessons acceptance cases L-FW3, L-FW9 and L-VR2 (ledger B10 allows
  these). The crate ceilings become **2,450 src and 1,830 tests**. Per-file ceilings for what is
  left: `main.rs` 200, `honesty.rs` 140, `legs.rs` 240, `history.rs` 155, `secrets.rs` 160 and
  `report.rs` 150; tests keep their slice 1 numbers. **Cut first:** slice 3 touches `main.rs` for
  `secrets`, and before it adds anything it brings `main.rs` back to 200 with the A9/B3 cut: dump's
  `--sid` arm becomes `Some(None) => return no_report(&"--sid takes N or BOOT.N")`, and `check()`
  shares one `now` binding. Same exit 3, same stderr prefix. The PRD-01 ask (about 40 lines of
  `input.rs`) is the next cut. A slice that would pass these numbers needs another decision line.
  PRD-16's end-of-build report nets this against the Rust totals, and only a test total past about
  14,200 becomes an owner question.
- **U1 is no reason to wait.** The slice 2 scope said `legs.rs` and `history.rs` wait for PRD-09's U1.
  U1's stub, with its final names, has been on integration since f5a529c8 (`usage/mod.rs`, 304 lines
  at 669f5d92; PRD-09 slice 1 note above). Those files can start against it now, in the order PRD-16
  sets. Only `readable()` as a method waits for PRD-09's core slice; the rule in that note covers
  either merge order.
- **For PRD-15:** this slice builds the three `server_*` ids (no longer "not built in this slice"
  under `--server`), adds the top-level `server` object and changes the exit rules above.
  `docs/RECORD-FORMAT.md` follows in PRD-15's next slice (PRD-15 note).
- Acceptance lists as known, if still open at the end: the A9/B3 `main.rs` nit and the PRD-01 ask.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-09 slice 2 (merge dab0ce1f)

- **`readable()` is a method on integration** (PRD-09 B10 done): `legs.rs` and `history.rs` read
  `table.readable()` and `thread.readable()`; no `readable` field exists. Nothing in PRD-08 read the
  field, so nothing changes on this side.
- **What U1 gives now:** wire rows for every call (kinds `Warmup` and `Interrupt` are rows, not
  calls; a rate limit sent as `error` plus `response.failed` is one row; a call cut by a restart or a
  GAP has `whole: false`), but `reason` is still `NoReader`, no threads and `children: None` until
  PRD-09's native slice. So on integration the rules already written hold: `harness_log` gives
  `none`, `subagents_present` (C2) is `skip`, and no check reads a wire-only table as a failure.
  `usage::run` still exits 3.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 3 (merge 7a995b6e)

- **Built and checked.** `secrets <path>... [--known <file>]` (new `src/secrets.rs`) and `check
  --known <file>`, which adds one `secrets` check (`inv` null) to every entry. Gate on 7a995b6e:
  `gate fast: PASS in 105s`, nextest 218 of 218 (goalcup-verify 36 of 36 in the lane's box runs).
  Six paths changed, all owned here, no `Cargo.toml`. The A9/B3 cut is done: `main.rs` is 200 with the
  `secrets` arm in it.
- **Section 3 "Secrets", as merged** (S12; the fixer's 16:27Z line asked the replan for this wording,
  it is no ledger item, so no W6 line follows):
  - **Deltas:** the joined text is the string values of `delta`, `text` and `partial_json` anywhere,
    and of `content`, `reasoning_content`, `thinking` and `arguments` inside a `delta` object (Chat
    Completions `choices[].delta` and its `tool_calls[].function.arguments`, Anthropic thinking).
    Gemini's `parts[].text` is covered by `text`. This replaces the bullet's three-key list.
  - **One joiner.** It is `secrets::delta_text(&Value, &mut String)`, public. Section 3.2 step 1's
    "the same joiner as the secret scan" means `history.rs` calls it; it never keeps a copy.
  - **Tails** are cut at any GAP of the stream (a GAP on sid 0 cuts all) and at a DATA offset jump, so
    nothing is joined across a hole (test `secrets_nothing_is_joined_across_a_gap`). Bytes are
    scanned per 64 KiB batch with the 4 KiB tail kept, and at END, GAP, jump and the end of the
    record (fixer's probe: 100,000 SSE pieces, 4.4 MB, `secrets` 174 ms).
  - **Inputs.** A part file is a record. A folder holding an entry has its record decoded and every
    other file read raw, except its `rec/` (the recorder's state, the run CA key among it), which is
    never read; a part folder of another name has its other files read raw. Other folders are walked.
    Any other file, and `-` (stdin), is raw bytes. An unreadable file inside a walk, or a folder
    symlink, leaves that input `partial` (exit 1). A path given on the command line that cannot be
    read, or an unreadable `--known` file, is exit 3 with no report. The section's "about 30 lines"
    for raw inputs is superseded by the size line below.
  - **The report** (stdout, always JSON; there is no `--json` flag): `verifier`, `result` (`pass`,
    `warn`, `fail`), `exit`, `known_values`, `inputs` (`[{path, as: record|raw, read:
    whole|partial}]`), `known_hits` (`[{form, in, sha12, n}]`), `shape_hits` and `glued_shape`
    (`[{kind, in, sha12, n}]`), `masks` (`{kind: span count}`, kinds named from the start event's
    `mask_rules`, `other(n)` when unnamed), `mask_bytes` (`{fail_closed: bytes}`) and `big_masks`
    (`[{kind, in, len}]`, a non-`fail_closed` span over 16 KiB, a `fail` by itself). Exit 2 on any
    known hit, unglued shape hit or big mask; else 1 when an input was read only in part; else 0
    (`warn` keeps 0). The `secrets` check of `check --known` has the same result rule and the same
    keys as facts.
  - A glued hit whose bytes just before it are an escape the masker's guard still masks (backslash,
    `\u`, `\x`, percent or ANSI escape, PRD-05 section 4) is not `glued_shape`. That list mirrors
    PRD-05's guard: a change there is a seam line and the same change in `secrets.rs` `ESCAPE`.
- **Size decision** (`wc -l` at 7a995b6e). Src without PRD-09's `usage/` is 2,235: `integrity.rs`
  982, `secrets.rs` 509, `server.rs` 201, `main.rs` 200, `input.rs` 198, `report.rs` 133, `lib.rs`
  12. That is seven of ten files, 160% of the PRD-00 row (1,400) and 91% of the 2,450 ceiling. Tests
  are 1,826: `tests/common/` 877, `check_invariants.rs` 329, `check_named.rs` 218, `secrets_scan.rs`
  214, `server_compare.rs` 188; 166% of the row (1,100) and 1,826 of the 1,830 ceiling. Past per-file
  ceilings: `secrets.rs` 509 of 160, `secrets_scan.rs` 214 of 30, `lib.rs` 12 of 11, `input.rs` 198 of
  150. With `honesty.rs` 140, `legs.rs` 240 and `history.rs` 155 the crate projects to about 2,770
  src; with `check_run1_shape.rs` 80 and `check_history.rs` 80, to about 1,986 tests. **Decision:**
  `secrets.rs` is capped at 509 (as merged) and `secrets_*.rs` at 215. The extra lines serve point 4
  (every known form, Chat Completions deltas joined, every loose file of an entry scanned, MASK spans
  merged and named by rule) and point 3 (no false hit across a GAP, no false C3 fail from the run CA
  key, a 100,000-piece stream in well under a second). The tests are point 4 and this PRD's
  acceptance (ledger B10). The crate ceilings become **2,730 src and 1,990 tests**: the src figure
  assumes the PRD-01 ask (about 40 lines of `input.rs`) is cut, which is now due before `legs.rs`
  merges; if PRD-01 declines it, a decision line nets the 40 elsewhere. Per-file ceilings for what is
  left stay: `honesty.rs` 140, `legs.rs` 240, `history.rs` 155, `check_run1_shape.rs` 80,
  `check_history.rs` 80. No further line in `secrets.rs`: a later secrets fix nets to zero there. A
  slice past these numbers needs another decision line. PRD-16's end-of-build report nets this
  against the Rust totals; only a test total past about 14,200 becomes an owner question.
- **Seam for PRD-12** (fixer 16:40Z): `secrets <entry>` reads the entry's `goalcup.log` and
  `upload.json` itself, so naming them again doubles their `n`. PRD-12 drops them from its argv
  (PRD-12 note).
- **Acceptance lists as known,** if still open at the end: review A5 nits 2 (`first: null`) and 3
  (`check --known` decodes the record a second time, besides the integrity pass); review items A7,
  A8 and F11, logged and not fixed (judge 16:32Z); a file passed both inside a walked folder and on
  its own counts its `n` twice under one key; the PRD-01 ask while open.
- **Later slices, unchanged:** honesty, legs, history and the run 1 shape builder, in PRD-16's order.
  `docs/RECORD-FORMAT.md` follows the new keys in PRD-15's next slice (PRD-15 note).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-09 slice 3 (merge 1a2c3537)

- **U1 now has native rows, threads and parents** (PRD-09 note). `legs.rs` and `history.rs` read
  them as merged: `CallTable.children` is `Some` once a native leg was read (`subagents_present`
  skips only on `None`); `Thread.readable()` is true only for a thread that owns a read session
  file, with one exception: a thread that only the wire names is listed with the table's own
  `reason`, so it is readable with `native_calls` 0. A check that asks whether a thread has native
  rows reads `native_calls`, never `readable()` alone. This changes no result today (wire-only
  threads are reported, not failed), and PRD-09 documents it in the next slice.
- A call joined by time has `join` `Matched` as one joined by id; `legs.rs` does not tell them apart
  (the count `joined_by_time` is in the summary only).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 4 (merge 1fdaf2c3)

- **Built:** `honesty.rs` with `listed_hosts_decrypted`, `conn_bytes` and `mask_failclosed`, fed one
  record at a time from integrity's single pass (`Pass::item` and `Pass::mask`); `report.rs` `LATER`
  now holds four ids: `history_complete`, `wire_keeps_up`, `threads_match`, `subagents_present`.
  A record of a stream never seen open is ignored by honesty (integrity reports it); state is
  dropped at END and at a new boot.
- **`conn_bytes` as merged** (PRD-03 and PRD-12 notes read it):
  - A decrypted connection is judged at its END, per direction, from the END tail `plain_up` and
    `plain_down` against what its streams account for: head sizes (approximate), END `wire` of each
    HTTP stream (`seen` when `wire` is absent), WebSocket DATA bytes, and a `conn` GAP's `lost - 1`.
    `fail` above 1 MiB and above 50% unaccounted with no GAP on the connection; `warn` above 1 MiB
    and above 10%.
  - **A stream GAP names only its own bytes.** An HTTP stream GAP adds nothing and never waives the
    fail rule for another request on the same connection (review A8). A WebSocket stream GAP adds
    `lost - 1` to what is accounted for; one of unknown size (`lost` 0) waives the rule for the
    connection. Only a `conn` GAP waives it otherwise.
  - A decrypted connection that ends with no `plain_up` or `plain_down` makes the whole check
    `skip` (facts `no_plain_tail`), as does a run with no decrypted connection ("no decrypted
    connection to judge"). A missing tail never reads as a pass.
- **`mask_failclosed`:** `warn` once a stream passes 1 MiB of `fail_closed` MASK bytes, counted per
  stream; never `fail`, so the exit stays 0. Facts `streams` and `bytes`.
- **`listed_hosts_decrypted`:** a listed tunnel is judged at its END: a sign-in is no loss, a GAP of
  its own reason is `named`, none is `fail`. A conn GAP on a decrypted listed connection is
  `named`. `warn` for a refusal after trust (unless its GAP tail says `alert: false`) and for a long
  tunnel to an unlisted host. The facts key `refusal_after_trust` is read by PRD-12, so it is a
  named seam key and the tests may assert it. `streams` is not read by anything outside the crate.
- **Size decision** (`wc -l` at 1fdaf2c3). Src without PRD-09's `usage/` is 2,678, 191% of the
  PRD-00 row (1,400) and 98% of the 2,730 ceiling: `integrity.rs` 991, `secrets.rs` 509, `honesty.rs`
  435, `server.rs` 201, `main.rs` 200, `input.rs` 198, `report.rs` 130, `lib.rs` 14. Tests (without
  PRD-09's `usage_run1.rs`) are 2,083, 189% of the row (1,100) and 93 over the 1,990 ceiling:
  `tests/common/` 877, `check_named.rs` 341, `check_invariants.rs` 329, `secrets_scan.rs` 214,
  `check_conn_bytes.rs` 134, `server_compare.rs` 188. `honesty.rs` is 435 against its 140 ceiling.
  **Decision:** `honesty.rs` is capped at 435, `check_conn_bytes.rs` at 134 and `check_named.rs`
  at 341 (as merged). The extra lines serve point 1 (a request lost on a reused connection must not
  pass: `conn_bytes` fails it, and a stream GAP cannot hide another stream's loss; a record that
  claims too little or too much must not panic, L-VR3), point 2 (a listed tunnel with no named GAP is
  a blind spot) and point 4 (fail-closed masking is counted and warned). The tests are the PRD-08
  acceptance cases (ledger B10). The crate ceilings become **3,040 src and 2,245 tests**: the src
  figure is 2,678 plus `legs.rs` 240 and `history.rs` 155 minus the PRD-01 cut of about 40 lines of
  `input.rs` (still due before `legs.rs`; if PRD-01 declines it, a decision line nets the 40
  elsewhere); the test figure is 2,083 plus `check_run1_shape.rs` 80 and `check_history.rs` 80. The
  per-file ceilings for `legs.rs` 240, `history.rs` 155, `check_run1_shape.rs` 80 and
  `check_history.rs` 80 stay. `integrity.rs` is 991 of 1,000: a hook for `legs.rs` or `history.rs`
  must fit in 9 lines or net against the file. A slice past these numbers needs another decision
  line. PRD-16's end-of-build report nets this against the Rust totals; only a total past about
  18,100 src or 14,200 tests becomes an owner question.
- **Review nits, disposition.** Fixed or accepted: A8 and its fix, B-R2, B-R3. Still open, taken up
  at the next touch of the file and listed as known if open at the end:
  - A9: in `check_conn_bytes.rs` the case `a_small_stream_gap_does_not_cover_a_silently_lost_request`
    ends the GAPped response with `seen` 2 while the loss runs to offset 12 (invariant 11), so the
    case fails `seen_matches_written` as well as `conn_bytes`. Fix: end that response with `seen`
    `2 + lost - 1`, and assert the failing ids are exactly `["conn_bytes"]`.
  - B-R6: reword the doc comment on `Conn.gaps` to say it counts conn GAPs and WebSocket stream GAPs
    of unknown size.
  - B-R7: `conn_gapped` picks the GAPped response by value; use `enumerate` and the last index. The
    `facts.streams` assert in `check_named.rs` pins a name only this crate reads; assert the result
    and exit code. `facts.refusal_after_trust` stays (PRD-12 reads it).
  - B-R1: OPEN meta, END tail and GAP tail are parsed twice (integrity and honesty). Declined for
    now (a two-file refactor for about 20 lines); it is the first cut if the crate nears its ceiling.
- **Build order, unchanged:** `legs.rs` (`wire_keeps_up`, `threads_match`, `subagents_present`),
  then `history.rs`, then the run 1 shape builder, in PRD-16's order. The PRD-01 `input.rs` cut comes
  before `legs.rs`. `docs/RECORD-FORMAT.md` and the check list in PRD-15's text follow the three new
  ids in PRD-15's next slice.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-09 slice 4 (merge c262ef5a)

- **`usage::run(args) -> ExitCode` is real** (`usage/out.rs`, `usage/mod.rs`); the `usage` arm in
  `main.rs` calls it and the exit-3 stub is gone. Exit 3 now means no entry gave output; a decode
  error partway gives the summary with `read_error` true and exit 0. No `main.rs` change is asked.
- **U1 grew by five public fields on `CallTable`:** `run`, `entry`, `connector`, `harness_version`,
  `sealed`, filled from the `start` event and the last PART. A table built with
  `CallTable::default()` plus field sets or struct update is unchanged, so `legs.rs` and `history.rs`
  need nothing; build tables the way the section "stub" rule says. No change to `CallRow` or `Thread`
  construction.
- **Thread readability, as merged:** a thread's `readable` is set from what the record or the native
  file proves; a thread seen only by a spawn is not marked readable. `subagents_present` keeps the
  rule in the earlier PRD-09 note (wire-only threads are not "present natively").
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-08 slice 5 (merge bfd244e3)

- **On integration:** `legs.rs` (`wire_keeps_up`, `threads_match` per call and totals, `subagents_present`
  from the record, `provider_errors`, `harness_log`, per-thread rows, the legs part of the coverage
  line), fed from integrity's one pass through `usage::Calls`; the run 1 shape builder
  (`tests/common/run1.rs`, `Shape::{Lossy, Vendor, Clean, Killed, Dodger, DodgerOwn}`) and
  `check_run1_shape.rs`. Left: `history.rs` with `check_history.rs` (slice 6); `timeline`, the
  `--harness-files` manifest part of `subagents_present` and the nits (slice 7).
- **Seams as built.** Covers: honesty collects them (`legs::Cover`: a `recorder_down` GAP, or a conn GAP
  of a connection to a provider or gateway host of the shared list). A GAP of a connection to a vendor
  host, or to a host only the participant or the config named, covers nothing, so run 1's shape stays
  `fail`. `recorder_down` alone removes both legs of a subagent. S6 `threads` rows are `{thread, parent,
  harness, wire_ok, wire_err, result, history: null}`; the entry-level totals row has `thread` null;
  `history` is filled by slice 6. `harness_log` is the string `none`, `totals` or `per_call`.
  `counts.threads` is the row count. `wire_keeps_up` hands its uncovered native times sorted to the
  sweep (the bug the shape builder found; do not drop the sort).
- **Slice 6 (`history.rs`, `check_history.rs`) adds to the shape tests.** `history_complete` joins
  `FAILS` in `check_run1_shape.rs` (a comment there says so) and the `history` field of the threads
  rows; `Clean`, `Killed` and the dodger shapes must keep it `pass` or `skip`. The case needs bodies
  whose later requests resend earlier answers; extend `run1.rs` `post` only as far as that needs.
- **Export for PRD-12, finish it in slice 6.** `GOALCUP_FIXTURES_OUT` is read by the Lossy test only and
  writes `k2-part` and `k2-fetched`. The pad lane also needs the Clean pair (`k7-part`, `k7-fetched`,
  for K6, K9, K10 in the selftest). `fetch()` makes `model_requests` one too high on purpose, so the
  Clean export needs an honest `fetch.json`: give `fetch` a flag, do not copy the lying one. Also
  write the K2 pair again after `history_complete` joins, since the failing set grows by one. The
  command is in the PRD-12 note. Ceilings: `check_run1_shape.rs` stays inside the decision below.
- **Size decision** (`wc -l` at bfd244e3, without PRD-09's `usage/` and `usage_run1.rs`). Src 3,241,
  231% of the PRD-00 row (1,400): `integrity.rs` 1,011, `secrets.rs` 509, `honesty.rs` 498, `legs.rs`
  477, `server.rs` 201, `main.rs` 200, `input.rs` 198, `report.rs` 133, `lib.rs` 14. Tests 2,627,
  239% of the row (1,100): `common/mod.rs` 879, `check_named.rs` 341, `check_invariants.rs` 329,
  `common/run1.rs` 232, `secrets_scan.rs` 214, `server_compare.rs` 188, `check_run1_shape.rs` 165,
  `check_legs.rs` 145, `check_conn_bytes.rs` 134. Against the slice 4 ceilings (3,040 src, 2,245
  tests, which counted `legs.rs` 240 and `history.rs` 155) the crate is 201 over on src before
  `history.rs`, and 382 over on tests before `check_history.rs`. `legs.rs` is 477 against 240,
  `honesty.rs` 498 against 435, `integrity.rs` 1,011 against 1,000, `run1.rs` 232 against 110.
  **Decision:** the lines serve point 1 (a loss of model calls on the wire, or a subagent with no leg,
  must not pass: the three leg checks, the covers, the totals form for Hermes), point 2 (every thread
  has a row and a verdict, `subagents_present` against the record's children), point 3 (a named loss
  is not a failure; a sweep over the rollout times that cannot stall a check) and the run 1 shape that
  PRD-12 K2 and PRD-16 N1 need (the shape builder is required by PRD-12, so it counts). The tests are
  PRD-08 acceptance cases (ledger B10). New ceilings: **3,460 src and 2,750 tests**. The src figure is
  3,241 plus `history.rs` 155, `timeline` and the manifest part about 60, a 9-line hook, minus the
  PRD-01 `input.rs` cut of about 40 (still not made: `input.rs` is 198, unchanged); tests are 2,627
  plus `check_history.rs` 80 and about 40 for slice 7. Per-file ceilings now: `legs.rs` 540,
  `honesty.rs` 498, `integrity.rs` 1,011 (a hook for `history.rs` nets against the file, so it ends at
  1,011 or less), `history.rs` 155, `check_history.rs` 80, `check_run1_shape.rs` 165, `check_legs.rs`
  145, `common/run1.rs` 232. A slice past these needs another decision line. If the `input.rs` cut is
  declined by PRD-01's owner, a decision line nets the 40 elsewhere; B-R1 (the doubled OPEN meta, END
  tail and GAP tail parse, about 20 lines) is the first cut if the crate nears the new ceiling.
- **Review nits, disposition.** New and taken up at the next touch of the file, else listed as known:
  - B-N4 (`honesty.rs`, `Honesty::open`): compute `listed` once as `s("listed").is_some() ||
    on.is_some()`, use it for `model` and for `Conn.listed`, and rename the local `kind` that shadows
    the `StreamKind` parameter. About 3 lines fewer, same behaviour. The `on_list` triple becomes a
    small private struct only if the code is touched again.
  - B-N5 (`tests/common/run1.rs`): re-wrap the doc comment on `fn tunnel` to rustfmt width (comment
    only).
  - Carried unchanged: A9, B-R6, B-R7 (and the `facts.streams` assert in `check_named.rs`), B-R1.
- **Build order:** slice 6 is `history.rs` with `check_history.rs`, the Clean export and the K2 re-export
  (above). Slice 7 is `timeline`, the manifest part of `subagents_present`, and the open nits. PRD-16 X1
  reads `timeline`; until slice 7 merges the pad and PRD-16 cannot use it.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-05 core slice (merge 2c95e8f1)

- **The stub caveat ends.** Entries built on the core slice have real MASK spans, `start.mask_rules`
  longer than `["fail_closed"]`, the real `mask_meta` and a `meta.masked` list on objects with headers.
  Checks and the secrets scan may read all four. S12 and the kind names are unchanged, so `shapes.tsv`
  readers and the `masks` map need nothing. `fail_closed` is the last entry of `mask_rules` and counts in
  `mask_bytes`.
- **Seam from the PRD-05 fix round (review A-2), rare in real traffic:** section 3's escape alignment can
  start a guarded-rule MASK on the escaping backslash (raw `x\sk-...`), so the byte before a MASK start is
  not always the byte before the shape match. The `glued_shape` test reads the bytes before the regex
  match and skips one escaping backslash; with that rule the escape list of the earlier note stands.
- **PEM over SSE:** inside a PEM block every JSON string interior is starred, keys included; lines outside
  the block are unchanged. A scan test that parses the masked line sees valid JSON with all-star strings
  between the markers.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, conductor seam line from PRD-02 core step 2 (08:50Z)

PRD-02's mask worker writes two record shapes PRD-01's text does not list yet; the verifier must accept them or every real entry fails inv4, inv12 or the recount: (a) a GAP with reason mask_failed, lost 1 (no stream byte lost) and tail {"meta": "open"|"tail"|"event"}, on the stream's sid or sid 0 for an EVENT, after a panic in mask_meta; it counts as one gap with 0 lost bytes and marks the request holed (any GAP on the sid), as PRD-02's Counter does; (b) an empty DATA with fin for an empty WebSocket message. PRD-08 accepts both in its next slice; PRD-01 notes them below.
