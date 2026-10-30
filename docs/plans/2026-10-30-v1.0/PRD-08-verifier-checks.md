# PRD-08: Verifier and checks

Id: PRD-08. Version: goalcup 1.0. Status: plan for the build. Date: 30 October 2026.
Revision 2, 30 October 2026: acts on every PRD-08 item of the consistency check and reviews A and B, and
follows PRD-00 and PRD-01 revision 2 (section "Review answers" at the end). Base: main at 4b226de2.
Revision 3, 30 October 2026: alignment fixes against the ledger (section "Alignment answers" at the end).
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md).

## Goal

One small `goalcup-verify` that reads GCR1, proves a record is whole, and fails loudly on every loss
that run 1 hid. Its JSON report (seam S6) is what the pad (PRD-12), the watchers (PRD-16) and organizers read.

## Points served

5 (the server copy is proven whole and rebuilt by ID and time), 3 (no silent loss: a loss without a
GAP is a failure), 1 and 2 (wire calls and subagents checked against the harness's own log), 4 (the
secret scan), 6 (the counts the participant saw equal the verifier's recount).

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
- T4: wire calls match the harness's own log, every subagent recorded, no secret, server copy whole.
  This PRD supplies those four; usage stats present is PRD-09's (PRD-12 C5).
- T8 and done item 1: Hermes passes "wire calls match" per call when its `state.db` rows carry usable
  times, else by a per-session comparison after the seal (PRD-00 question 7, default; section 3.1).

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
planted values, canary), 0600, deleted after the scan. The conductor runs only `fetch-run`, sandbox
control and git.

The judge view (`goalcup-verify judge`) is not in this build (PRD-00 "Not in this build", review B N2).
Its one fact judges need now, the evidence label, is a key in S6 (section 5).

Inputs `check` takes so it can compare the record with the harness's own log:

- The record itself, always. Native session files are in it (PRD-04), so the harness's own log is
  read from the record, not from the disk. That makes the check work on the server copy.
- `--harness-files <manifest.json>` (pad only): the pad lists files on disk under the connector's
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
| 1 to 9 | `inv1` to `inv9` | inv4 counts from `meta.from` and exempts `conn` streams; inv6 is boot order (one `recorder_down` per boot after 0); inv7 is "every COUNT equals a recount" |
| 10 | `streams_closed` | In an unsealed or live entry, open streams are listed and the result is `named`, not `fail` |
| 11 | `seen_matches_written` | A stream with an unknown-loss GAP is `named` with the hole's size, not checked |
| 12 | `gap_has_end` | A GAP with start equal to end and `lost` 0 or 1 is run 1's shape: `fail` |
| 13 | `recorder_alive` | COUNT at most 180 s apart within a boot, after taking off `suspend` time. With `--live`, also: the last COUNT is under 180 s old by the wall clock now, minus any sleep the record shows |
| 14 | `inv14` | Native resume has no hole across boots |

Also exit 2: a torn tail (a record cut inside its length) at the end of a sealed entry; two streams of
one boot with the same native `path` and `gen` (PRD-04 interfaces row). A torn tail at the end of a
live or crashed entry is the open end PRD-01 section 12 expects. GAP reasons 8 and 9 (reserved,
never written by 1.0) and any unknown number show as `other(n)` with `warn` (`gap_reason_unknown`).

**Honesty: the checks that must fail run 1's shape.**

| Id | Rule | Pass | Run 1's shape fails because |
|---|---|---|---|
| `wire_keeps_up` | Harness calls (native leg, U1 rows with times) against wire calls whose outcome is not an error, cumulative over time. Fail when, at any moment T, harness calls up to T exceed wire calls up to T plus 2 minutes by more than max(3, 2%), and the excess is not inside a named GAP, off span, a named tunnel (GAP 1 for `hello_too_big`, 6, 7, 13 or 14) to the harness's model host, or a `conn` GAP (for example 11) on a decrypted connection to it. A harness call is covered when its time lies from the GAP's `start_wall` to its `end_wall` plus the 2-minute lag. A GAP that fails `gap_has_end` covers nothing. Also fail when the harness's last call is more than 10 minutes after the wire's last call with no such cover (`wire_ended_before_native`). Wire above harness is normal (retries, warm-ups) and only reported. `skip` when U1 has no per-call times for this connector (section 3.1) | Never harness above wire, unexplained | Wire stops 15:43:13Z; 473 harness calls follow to 16:54Z, 71 minutes, no cover |
| `threads_match` | Per thread (U1 thread ids: Codex `thread-id` header and `client_metadata`, Claude Code subagent files, and so on): harness calls not above non-error wire calls, by the same tolerance. A thread with harness calls and no wire call fails, unless covered as above. A wire-only thread is reported. For a harness whose log has totals only, the totals form of section 3.1 | Every thread matches or is explained | Root thread 358 wire against 591; 15 threads have harness calls and no wire call |
| `subagents_present` | Every child a parent names (U1 `children`: Codex `receiver_thread_ids`, `crates/goalcup-verifier/src/capture_analysis/native/codex.rs:418-427`; Claude Code `subagents/agent-*.jsonl`; Hermes `sessions.parent_session_id`) has a native stream or wire calls, or sits inside a named gap. With `--harness-files`: every append file on disk has a `native` stream whose last END `seen` equals its size, or a named GAP; every database or snapshot file has a `snapshot` stream (sizes not compared: a database keeps growing) | Every subagent has a leg | 11 children have neither leg; 11 rollout files have no native stream |
| `listed_hosts_decrypted` | Every `conn` to a host on a list (`listed` not null) is `decrypt`; or a `sign_in` tunnel (no loss, `pass`); or a tunnel whose GAP names its reason (1 `memory_guard` for `hello_too_big`, 6, 7, 13, 14: `named`, PRD-00 ruling, PRD-01 section 4). A decrypted `conn` with a `conn` GAP after a parser fault (4, 11 or 1) is `named`. A listed tunnel with no GAP is `fail`. Two `warn` facts: a `long_tunnel` note on an unlisted host (a possible model host to add), and `refusal_after_trust`: a host and ClientHello `fingerprint` pair that completed a decrypted connection earlier in the entry and later got a `tls_refused` tunnel (review A finding 1: the harness itself may be tunnelled by a learned refusal). A `tls_refused` GAP with tail `{"alert": false}` is skipped by `refusal_after_trust`: nothing was learned from it (PRD-03 section 1) | No listed host relayed blind | Passes (no model host was tunnelled in run 1) |
| `conn_bytes` | Per decrypted `conn`, each direction: `plain_up` or `plain_down` (PRD-01 END tail) against what its streams account for: request and response heads (the OPEN `headers` size, approximate), HTTP body bytes on the wire (END tail `wire`), WebSocket message bytes, and any `conn` GAP's `lost - 1`. `warn` when more than 10% and more than 1 MiB is unaccounted. `fail` when more than 50% and more than 1 MiB is unaccounted and no `conn` GAP covers the connection (review A finding 2: with PRD-03's `conn` GAP after a parser fault, unaccounted bytes on that scale can only be a silent loss) | Bytes add up within the stated slack | Not applicable to run 1 (v5 had no such tails); it guards 1.0's reused-connection fault |
| `mask_failclosed` | `warn` when one stream has more than 1 MiB of `fail_closed` MASK bytes, with the stream count and the total (COUNT `mask_failclosed_bytes`, review A finding 3) | No large fail-closed span | Not applicable |

A note on inflate: WebSocket messages are recorded inflated, so a deflate connection can account for
more bytes than went over it. That only makes `conn_bytes` quieter; it never fails a good run.

#### 3.1 Which harnesses have a per-call log

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
`skip` with "could not read the harness's own log". The pad maps a `skip` here to INCONCLUSIVE
(PRD-12 C1).

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
   "threads":[{"thread":"…","parent":"…","harness":591,"wire_ok":358,"wire_err":40,"result":"fail"}]}],
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
- **One coverage line** in the text output, for example: "The harness's own log: 1,261 calls by 43
  threads. Recorded: 914 requests, 97 refused by the provider. Harness calls with no recording: 473,
  from 15:43 to 16:54 (71 min). Recording off: 0 times, 0 s." The seconds are the `off_ms` total.
  When no harness reader exists for the
  connector: "The harness's own log is not read for this harness", never "everything recorded".

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
  `src/server.rs`, `src/secrets.rs`, `src/report.rs` (S6, text and the coverage line).
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
| PRD-02 | GAP tail keys `guard`, `limit`, `cause`, `prev`, `exits`, `last`, `conns`, `hosts`; exit codes 70, 73, 74, 75 as `recorder_down` causes (shown in the text output); COUNT every 60 s. No test hook is used here | Used here |
| PRD-03 | `conn` meta `listed`, `kind`, `mode`, `reason` (incl. `hello_too_big`), `fingerprint`; the `conn` GAP after a parser fault; notes `long_tunnel`, `parse_stopped`; the `tls_refused` GAP tail `{"alert": false}`. Every named tunnel on a listed host (GAP 1 for `hello_too_big`, 6, 7, 13, 14) and every `conn` GAP after a parser fault (for example 11) is `named`; `sign_in` is `pass` | Used here |
| PRD-04 | Native meta `path`, `gen`, `from`, `fid`; snapshot meta `set`, `form`, `harness_exited`; END `source_changed` with no GAP allowed; GAP tail `unread_at_least`; one path and `gen` per boot; periodic database copies (the last whole `set` is the one U1 reads) | Used here |
| PRD-05 | S12: `src/mask/shapes.tsv` (kind, guard, regex), read with `include_str!` and compiled without the guard; rule kinds in `mask_rules`; only `fail_closed` spans may exceed 16 KiB | Used here |
| PRD-06 | `upload.json` `keep`; `GOALCUP_KEEP_PARTS=1` for `server_equals_local`; the `recording` rule from counts, which `server_counts_match` recomputes | Used here |
| PRD-07 | S13: `<outdir>/<entry>/rec.gcr.zst` and `fetch.json` (chunk list with keys and SHA-256, and the entry row: `model_requests`, `upload_offset`, `final_seq`, final head, the last heartbeat's `recording`, sealed time), each chunk's compressed size (`bytes`) and the last heartbeat's `counts` | Used here |
| PRD-09 | U1, owned by PRD-09 and used as is: `usage::Calls::new(&Start)`, `push(&Record)`, `finish() -> CallTable` with rows (leg, entry, boot, sid or path, thread, parent, wall times, kind, outcome incl. `RateLimited`, `Cancelled`, `whole`, model) plus `threads`, `readable`, `children`; `native_form`, per-session `api_call_count`, and per thread `native_only`, `native_until`, `readable`, `reason`. Also `usage::run(args) -> ExitCode` | Used here |
| PRD-10 | The `goalcup-verify check` line goes to `goalcup.log` only, when local part files are whole; otherwise it names the server copy. The participant's counts are the ones `counts_match_seal` recounts | Used there |
| PRD-11 | Connector name and the `start` event `hosts` decide the harness reader and `listed_hosts_decrypted` | Used here |
| PRD-12 | S6; the `--harness-files` manifest and `--known` file the pad writes and copies into the box where the checks run (section 1 "Where it runs"). C1 = `wire_keeps_up` + `threads_match`; C2 = `subagents_present`; C3 = `secrets`; C4 = integrity checks and `server_*`. Dodger case: exit 1, from the named `pinned` tunnels (`wire_keeps_up` is `named` where it runs, `skip` where U1 has no per-call times). K2 uses this crate's run 1 shape builder | Used there |
| PRD-13 | Deletes `crates/goalcup-verifier/` after this PRD's tests pass. Size numbers that replaced `research-measure.py` are S6 `bytes` | Done there |
| PRD-14 | `cargo nextest` covers `crates/goalcup-verify` in `gate fast`; the release ships `goalcup-verify`. Test list: the six files under Acceptance (no `judge_view.rs`) | Used there |
| PRD-15 | RULES "What a label is worth" and the CHEAT-CHECKLIST cross-leg text rewritten for S6 `label` and the coverage line; the judge view is "after the merge" | Written there |
| PRD-16 | Watchers run `check --live` every 5 minutes (hourly for a week-long entry) in the pad box via `sandbox exec` under `nice -n 19`, and `check --server` after each run in the run's pad box or a fresh check box (ledger B1); a `fail` goes in the night log. Tolerances as stated here until PRD-16 changes them with evidence | Used there |
| PRD-17 | H10 and K15 of PRD-17 write a part file through the real pipeline; their test also runs `goalcup-verify check` on it and asserts exit 0. No other lessons test writes one | Used there |

## What is cut

| Path | Lines (src / tests) | Tests that go |
|---|---|---|
| `crates/goalcup-verifier/` (whole crate, section 6) | 13,488 / 20,724, plus 2,182 fixture lines | All of `tests/` |
| Flags `--capture-integrity`, `--research-json`, `--stats`, `--dev-key` | in the above | `v5_corpus.rs` exit-code cases |
| The research report NDJSON format (`docs/research/m2-report-format.md` is PRD-15's to retire) | in the above | `m2_research_report.rs`, byte goldens in `tests/fixtures/m2-report-bytes/` |
| Not built in 1.0: the judge view, the run 1 converter, the silent-drop hook proof | 0 | none written |

New code: about 1,430 src lines (input 150, integrity 380, honesty 150, legs 300, server 130, secrets
230 with raw inputs, report and main 100) and about 1,100 test lines. PRD-00 budgets 1,500; the judge view's 150 is
out, the totals form, `conn_bytes` fail rule and `fetch.json` reading are in.

## Acceptance

### Real-run proofs (first)

| # | Proof | Pass |
|---|---|---|
| R1 | **Run 1's shape fails, on the synthetic record and on 1.0 itself.** (a) `check_run1_shape.rs`'s record written as a part folder and as a fake fetched copy (PRD-12 K2). (b) A1, the 2-hour Codex run on 1.0 (PRD-16), compared with run 1's table | (a) Exit 2; exactly `gap_has_end`, `streams_closed`, `wire_keeps_up`, `threads_match`, `subagents_present` and `server_counts_match` fail; `provider_errors.wire` reads the planted count once. (b) Exit 0 or 1, never 2; no 10-minute bucket with harness calls and 0 wire calls unless covered by a named GAP; `provider_errors.wire` equals, within one per call, the failed calls in Codex's own rollouts (U1 native errors, listed apart) |
| R2 | **A clean run passes.** Claude Code, `direct`, `full` (PRD-12) | Exit 0, or exit 1 where the only `named` result is an `after_stop` GAP. Every other check `pass` or `skip`. `threads_match` covers the main thread and every subagent. `server_equals_local` passes. `secrets` finds 0 known values |
| R3 | **A killed recorder is named, not failed.** Codex kill test (PRD-01 acceptance 3, PRD-02 R1) | Exit 1. Two `recorder_down` gaps. Every harness call missing from the wire falls inside them, so `wire_keeps_up` and `threads_match` are `named`. No `fail` |
| R4 | **The dodger case is honest.** Codex with its model host marked `pinned` on the pad | Exit 1, never 0 or 2. Every connection to that host is a tunnel with GAP `pinned`; `listed_hosts_decrypted` is `named`; `wire_keeps_up` is `named` by them; the label is `native_only` |
| R5 | **Hermes passes, per call or by totals.** Hermes, DeepSeek key, one delegated child (PRD-12 core row) | `harness_log` is `per_call` or `totals`, as PRD-09 reads it. With `per_call`, `wire_keeps_up` and `threads_match` pass as for Codex. With `totals`, `wire_keeps_up` is `skip` "no per-call times" and `threads_match` passes in the totals form (per session, or per entry when wire calls carry no session id). Either way `subagents_present` finds the child from `parent_session_id` |
| R6 | **Load.** PRD-16 A1 (Codex, 2 hours) and A2 (Claude Code, up to 20 subagents, 60 minutes, one recorder kill), as PRD-16 runs them under the 10 USD cap (PRD-03 acceptance 10) | `threads_match` passes or is `named` for every thread (A2's kill window is named by its `recorder_down` gap); `provider_errors` shows any 429s. Time for `check` is logged as a measure, never a pass line (PRD-00: no speed number fails a gate) |
| R7 | **Secrets.** The pad's planted `.env` and prompt (PRD-05 A1), all three cases | 0 known values in local part files and server copy; every planted shape has a MASK |

The run 1 converter is not built, so run 1's own server copy is not re-checked (PRD-00 "Not in this
build"). Run 1's numbers stay in the run report.

### Automated tests that guard them

- `check_invariants.rs`: a valid entry built with `PartWriter` exits 0; one mutation per invariant
  (1 to 14), a torn tail, a repeated native `path` and `gen` in one boot, and an unknown record kind
  give the right id and exit; GAP reason 8 gives `warn`, not a crash.
- `check_run1_shape.rs`: a small record shaped like run 1 (root and four Codex-shaped threads, wire
  stops at minute 49 while rollouts grow to minute 120, six rate-limit errors sent both as an `error`
  event and a `response.failed`, three GAPs with start equal to end and unknown loss, two streams with
  no END in a sealed entry, one child with neither leg). Pass: exactly those checks fail, exit 2,
  `provider_errors` reads 6. The same record with the loss removed exits 0, so the checks do not fire
  on a clean run. It runs against PRD-09's real U1 sink once that lands (a stub before).
- `check_named.rs`: the clean record with one `recorder_down` gap that covers the missing calls: exit 1,
  results `named`. Also: a listed tunnel with each GAP 6, 7, 13, 14, and a `hello_too_big` tunnel with
  GAP 1, is `named`; with no GAP, `fail`; an `after_stop` GAP is `named`; a Hermes-shaped totals record
  passes `threads_match`; a `tls_refused` GAP with `{"alert": false}` raises no `refusal_after_trust`.
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
  carry usable times, else the totals form. Every other harness gets `skip` on the leg checks until a
  pad sample adds a reader. Then a silent loss is caught
  only by the integrity checks, `conn_bytes` and `recorder_alive`, and the text output says the log
  was not read. The totals form cannot say when a loss happened, only that one did.
- **Hermes wire calls may carry no session id.** Then the totals form compares per entry, and a lost
  child is visible only if the sum falls short.
- **A loss before a stream opens is invisible.** If the proxy drops a whole connection without a
  `conn` record, only the harness's log can show it. For a harness with no session files, nothing can.
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
   when PRD-09's Hermes reader exists (second slice).
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
`nice -n 19`: a `fail` on `wire_keeps_up` or `recorder_alive` is the alarm run 1 never raised.

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
