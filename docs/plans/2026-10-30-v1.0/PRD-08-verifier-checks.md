# PRD-08: Verifier and checks

Id: PRD-08. Version: goalcup 1.0. Status: plan for the overnight build. Date: 30 October 2026.
Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md).

## Goal

One small `goalcup-verify` that reads GCR1, proves a record is whole, and fails loudly on every loss
that run 1 hid. Its JSON report is what the pad (PRD-12), the watchers (PRD-16) and the judges read.

## Points served

5 (the server copy is proven whole and rebuilt by ID and time), 3 (no silent loss: a loss without a
GAP is a failure), 1 and 2 (wire calls and subagents checked against the harness's own log), 4 (the
secret scan), 6 (the counts the participant saw equal the verifier's recount).

## Owner decisions it carries out

- Verifier and Worker move to the new format in the same build; no backward compatibility, no v5
  reader (BRIEF3, 29 Oct evening).
- Loss can never be silent. Any loss leaves a gap with a start and an end (handover ask 3, BRIEF2).
- Count a model call only once its bytes are in the record; show how long recording was off
  (handover ask 4). The participant, status, heartbeat and server see only written counts.
- No caps of our own: the old reader's caps (128 open streams, 128 connections) are cut.
- Research decoders are cut, except the model and usage parts PRD-09 keeps.
- Signing, receipts, acks, baseline and seal manifests, termination proof are cut (PRD-00 question 4).
- Record raw, interpret later: everything here runs after the run, on the organizer's or the pad's
  machine, never in the recorder (AGENTS.md, D3).
- The test pad checks "wire calls match the harness's own log, every subagent recorded, no secret,
  server copy whole" (BRIEF3). This PRD supplies those checks.

## Today

| What | Where | Fact |
|---|---|---|
| Crate size | `crates/goalcup-verifier/` | 13,488 src lines, 20,724 test lines, 2,182 fixture lines (`wc -l`) |
| Three modes | `crates/goalcup-verifier/src/bin/goalcup-verify.rs:44-71` | `--capture-integrity`, `--research-json`, `--stats`, plus `--dev-key` for signing |
| Exit codes | `crates/goalcup-verifier/src/capture_analysis/reader.rs:269-281`; `goalcup-verify.rs:296-310` | 0 valid, 1 incomplete or unsealed, 2 invalid, 3 no usable report (usage errors too) |
| Gaps never fail | `reader.rs:1126-1141` | `gap_recorded`, `producer_recovered`, `producer_unclosed` only make the report "incomplete" (exit 1) |
| A hole needs a gap | `reader.rs:1726-1733` | `segment_source_unexplained_hole` unless a gap is pending on that stream: the one silent-loss rule today |
| Caps as verdicts | `reader.rs:78, 1384, 1566, 1694` | More than 128 open streams, connections or goal streams makes a trace invalid (exit 2) |
| Cross-leg rule | `crates/goalcup-verifier/src/capture_analysis/report.rs:376-386` | Judges only zero against non-zero. `wire_last` exists (`session.rs:193-197`) but is never compared with the native leg |
| Provider errors | `crates/goalcup-verifier/src/capture_analysis/stats.rs:949-956, 1023` | Errors come from the same leg as the tokens. In run 1 that was native, so the wire's rate-limit errors read 0 |
| Double count trap | `crates/goalcup-verifier/src/capture_analysis/openai.rs:475-486, 545-558` | An `error` stream event and a `response.failed` each make an error observation |
| Evidence label for judges | `report.rs:391-401`; `docs/RULES.md:306-319` | `wire_and_native`, `wire_only`, `wire_undecoded`, `native_only`, `none` |
| Producer lineage | `crates/goalcup-verifier/src/capture_analysis/lineage.rs:1-142` | Recovery supersession of proxy producers. Producers do not exist in 1.0 |
| Who uses it | appendix C section 8.2 | Nothing on the recording path. `goalcup doctor` (`crates/goalcup-cli/src/commands/doctor.rs:246-316`), `scripts/matrix.sh:2054-2105`, `scripts/linux-harness-smoke.sh:231`, the organizer after `scripts/fetch-run.sh` |

What run 1's server copy got from today's verifier (run report, "Server copy", section 1): every mode
exit 1, never 2. `cross_leg.state` was `consistent` although the wire stopped at 15:43:11.9Z and the
native leg ran to 16:54:43Z. `provider_errors.total` was 0 although the record holds about 97
`rate_limit_exceeded` error events and 99 `response.failed`. Only `producer_unclosed` hinted at the
loss. The 23 gaps had one time each and `lost_bytes: null`. The server stored `model_requests` 1,766
against 842 calls in the record.

## Design

### 1. One binary, five subcommands

`crates/goalcup-verify`, binary `goalcup-verify`, written fresh against `goalcup_format` (F1). It links
`goalcup-format` and its own `usage` module (PRD-09). It never links the recorder crate.

| Command | Does | Used by |
|---|---|---|
| `check <dir>` | Integrity, honesty and count checks on an entry folder, a run folder (all entries in order) or a fetched server copy. Text to stderr, `--json` report (seam S6) to stdout | Pad, watchers, organizers, participant |
| `check --server <fetched> [--local <entry dir>]` | Also proves the server copy whole, equal to the local parts, and its database row equal to the seal | Pad, organizers |
| `check --live <entry dir>` | The same, on an entry still recording: reads to the durable point (last index line), tolerates the open frame, and also checks that the recorder is writing now | Watchers during the 2-hour run |
| `dump <dir>` | One JSON line per record (PRD-01 `dump`); `--sid`, `--kind`, `--text` filters | Builders, watchers |
| `secrets <dir> [--known <file>]` | The secret scan. Reports counts and hash prefixes only | Pad, organizers |
| `judge <run dir>` | The judge view: one plain page per run, plus `--json` | Judges, organizers |
| `usage <dir>` | PRD-09's usage reader; dispatch only here | Organizer job |

Inputs `check` takes so it can compare the record with the harness's own log:

- The record itself, always. Native session files are in it (PRD-04), so the harness's own log is
  read from the record, not from the disk. That makes the check work on the server copy.
- `--harness-files <manifest.json>` (pad only): the pad lists files on disk under the connector's
  native roots, as `[{"path": "<source>:<relative path>", "size": n}]`. The check then also proves
  that every file on disk has a native stream. The verifier never walks a home folder itself.
- `--known <file>` (pad only, 0600): real and planted secret values, one per line, for the scan.

Exit codes keep today's meaning (`reader.rs:269-281`): **0** whole, every check passed; **1** honest
but incomplete: a loss that is fully named by GAPs or off spans, an unsealed or live entry, or local
parts pruned; **2** a failure: a broken invariant or a silent loss; **3** no usable report: the
input cannot be read, or the command line is wrong. `--help` and `--version` exit 0. Copy
`goalcup-verify.rs:296-310` for the parse rule and `:264-294` for safe output files (new file, 0600,
outside every input folder).

### 2. The reader

One streaming pass over the record stream: concatenated parts (or fetched chunks), decoded once with
a 2^27 window (`goalcup_format::read`). No caps of our own. State per open stream is a few words
(kind, conn, req, next offset, open gap span, last time), dropped at END. Per thread and per 10-minute
bucket, a few counters. Memory is the 128 MiB window plus that state. No memory guard is needed: this
is an offline tool, and a week-long run with thousands of subagents holds at most a few thousand open
streams at once.

Boots replace producers. `lineage.rs` goes. Boot order is invariant 6 (one `recorder_down` per boot
after 0), about 30 lines.

A finding is a code plus counts, never bytes. Distinct codes are kept as in `reader.rs:442-476`
(copy the idea). Each check keeps its first failure with raw offset, `sid` and wall time, so a person
can `dump --sid` to it.

### 3. The checks

Every check has one id, one pass rule and a result: `pass`, `named` (loss present, and every lost
part sits inside a named GAP or off span), `warn`, `skip` (with why), or `fail`. `fail` gives exit 2,
`named` gives exit 1. `warn` and `skip` never change the exit.

**Integrity** (PRD-01 section 13, one id each, `inv1` to `inv9`). Any failure is exit 2. Plus a
torn tail (a record cut inside its length) at the end of a sealed entry is exit 2, and at the end of
a live or crashed entry is the open end PRD-01 section 12 expects.

**Honesty: the checks that must fail run 1.**

| Id | Rule | Pass | Fails run 1 because |
|---|---|---|---|
| `gap_has_end` | Every GAP has `end_mono` after `start_mono` and `end_wall` not before `start_wall`, or a known `lost` (above 0). A GAP with start equal to end and unknown loss is the run 1 shape: a loss with no extent | Every GAP has an extent | All 23 gaps had one time and `lost_bytes: null` |
| `streams_closed` | In a sealed entry every stream has an END. A stream of boot b with no END is allowed only when boot b+1 starts with a `recorder_down` gap (PRD-01 section 7). In an unsealed entry, open streams are listed and the result is `named` | No stream left open | 18 proxy producers never closed; their open streams have no end (count unverified until converted) |
| `seen_matches_written` | For every END: `seen` equals the stream's last written offset plus the bytes of its GAPs with known loss. Unknown-loss GAPs make it `named` | Equal for every stream | `skip` on converted run 1 (v5 has no `seen`); it is the 1.0 guard against a proxy that relays bytes it never offers to the writer |
| `recorder_alive` | Within each boot, COUNT events at most 180 s apart until its seal or its last LINK. With `--live`, the last COUNT is under 180 s old | No silence | Passes on converted run 1 (the daemon wrote); it exists for the 1.0 shape of the same fault, a stuck writer |
| `wire_keeps_up` | Harness calls (native leg, from PRD-09) against wire calls written with a non-error outcome, cumulative over time. Fail when, at any moment T, harness calls up to T exceed wire calls up to T plus 2 minutes by more than max(3, 2%), and the excess is not inside a named GAP, off span or `pinned`/`tls_refused` tunnel of the harness's model host. Also fail when the harness's last call is more than 10 minutes after the wire's last call with no such cover (`wire_ended_before_native`). Wire above harness is normal (retries, warm-ups) and only reported | Never harness above wire, unexplained | Wire stops 15:43:15Z; 473 harness calls follow to 16:54Z, 71 minutes, no cover |
| `threads_match` | Per thread (PRD-09 thread hints: Codex `thread-id` header and `client_metadata`, Claude Code subagent files, and so on): harness calls not above wire calls with a non-error outcome, by the same tolerance. A thread with harness calls and no wire call fails. A wire-only thread is reported | Every thread matches or is explained | Root thread 358 wire against 591; 15 threads have harness calls and no wire call |
| `subagents_present` | Every thread a parent names (Codex `receiver_thread_ids`, `crates/goalcup-verifier/src/capture_analysis/native/codex.rs:418-427`; Claude Code `subagents/agent-*.jsonl`) has a native stream or wire calls, or sits inside a named gap. With `--harness-files`: every file on disk has a native stream whose last END `seen` equals its size, or a named GAP (PRD-04's pad check) | Every subagent has a leg | 11 children have neither leg; 11 rollout files have no native stream |
| `listed_hosts_decrypted` | Every `conn` to a host on a list (`listed` not null) is `decrypt`, or a tunnel with GAP `pinned`, `tls_refused` or `h2_unsupported`. A `long_tunnel` note on an unlisted host is `warn`: a possible model host to add | No listed host relayed blind | Passes (no model host was tunnelled in run 1) |

**Reports that never fail on their own.**

| Id | What | Why |
|---|---|---|
| `provider_errors` | Wire calls whose outcome is an error, by code: HTTP status from `http_resp` meta (429, 4xx, 5xx, no body parse), and body codes from PRD-09 (`rate_limit_exceeded`, `overloaded_error`, `response.failed`). One per call: a stream `error` event counts only when no `response.failed` shares the call. Native-leg errors are listed apart | Run 1 must read 97 to 99 wire errors, not 0 and not 196. They also feed `wire_keeps_up` and `threads_match`, which compare non-error wire calls |
| `counts` | The recount: model requests written (`http_post` + `ws_up_msgs`, PRD-01 section 9), whole and holed requests, gaps and lost bytes by reason, off time, tunnels, native files and bytes, per boot and per entry. Equal to every COUNT (that is `inv7`) and to the seal totals (`counts_match_seal`, exit 2 if not) | The number the participant saw is the one the verifier recounts (PRD-00 done item 1) |
| `timeline` | Per 10 minutes: harness calls, wire calls, wire errors, native bytes, gaps, off time | Run 1's reconcile was read this way (appendix M section 5.6); the judge view prints it |
| `conn_bytes` | Per decrypted `conn`: plaintext bytes each way against the bytes of its request, response and WebSocket streams. `warn` when more than 10% and more than 1 MiB is unaccounted | Catches a proxy that relays without offering copies. `warn` until three pad runs show the real overhead; then PRD-16 may make it `fail` |

**Server copy** (`check --server`).

| Id | Rule | Result |
|---|---|---|
| `server_chunks_whole` | Every chunk's SHA-256 equals its key's; offsets run on with no hole from 0; link `seq` runs on; each chunk's `prev` equals the previous chunk's head; the last chunk ends at the final heartbeat's `seq` and hash | `fail` on any break |
| `server_equals_local` | With `--local` and parts kept (`GOALCUP_KEEP_PARTS=1`, PRD-06): SHA-256 of the concatenated chunks equals that of the concatenated local parts, up to the shorter of the two | `fail` on difference. A pruned local copy is `skip` with "use the server copy" |
| `server_counts_match` | The entry's database row (from PRD-07's fetch output): `model_requests` equals the seal's written total; `upload_offset` and `final_seq` equal the last chunk; `recording` is not `ok` when the record has any GAP or off span | `fail` on difference |

**Secrets** (`secrets`, and inside `check` when `--known` is given).

- Per stream, DATA pieces are joined in offset order with a 4 KiB overlap, so a secret split across
  two records is found (run 1's observer missed this, appendix M section 5.6 lesson 3). Meta values
  and event JSON are scanned too.
- Known values (`--known`): raw, base64 in all three alignments, percent-encoded, JSON-escaped.
- Shapes: PRD-05's rule table **without** the left word guard (PRD-05 interfaces row PRD-08), so a
  key glued to a letter is flagged even though the masker accepts missing it.
- Deltas: for SSE and `ws_down` JSON text, the string values of `delta`, `text` and `partial_json` are
  joined in order per stream and scanned, because a key the model repeats across deltas is not masked
  (PRD-05 line 326).
- Masks: no MASK span over 16 KiB (a masked request tail, run 1 plan T17); spans counted by rule.
- Output: hits by kind and stream, with the first 12 hex digits of each hit's SHA-256 (run 1's
  method). Never the value. Any hit of a known value is `fail`; a shape hit is `fail` unless the pad
  lists it as expected.

### 4. Seam S6: the `check --json` report

Stable by H+3. The pad and the watchers read only these keys.

```json
{"verifier":"1.0.0","input":"local|server|live","run":"r…",
 "result":"pass|named|fail","exit":1,
 "entries":[{"entry":"e…","sealed":true,"seal_reason":"harness_exit","boots":2,
   "raw_bytes":0,"links":0,"connector":"codex-cli",
   "checks":[{"id":"wire_keeps_up","result":"fail",
              "why":"harness above wire by 473 calls from 15:43:15Z to 16:54:43Z (71 min)",
              "first":{"o":123,"sid":0,"wall":"…"},"facts":{}}],
   "counts":{"model_requests":914,"req_whole":0,"req_holes":0,"gaps":{"memory_guard":0},
             "lost_bytes":0,"off_ms":0,"tunnels":0,"native_files":0,"threads":0},
   "provider_errors":{"wire":{"rate_limit_exceeded":98},"native":{}},
   "threads":[{"thread":"…","parent":"…","harness":591,"wire_ok":358,"wire_err":40,"result":"fail"}]}],
 "server":{"chunks":20,"whole":true,"equals_local":null,"db_match":false}}
```

`why` is plain English, one line, no secret, no path outside the record. Unknown keys are ignored by
readers. The text form on stderr prints `why` per failed or named check.

### 5. The judge view

`goalcup-verify judge <run dir>` replaces the research report's evidence label and `cross_leg` note
(`report.rs:334-401`, `docs/RULES.md:306-319`, `docs/CHEAT-CHECKLIST.md:44-47`). It reads a fetched
run (all entries) and prints one page:

1. Run, entries, boots, start and end (server time), seal reasons, workspace head and dirty digest.
2. **Label**: `wire_and_native`, `wire_only`, `native_only` or `none`, from written model requests and
   native streams with bytes. `wire_undecoded` goes: whether PRD-09 could read a body is shown on the
   next line, not in the label. The label stays a hint for judges, not a judgement.
3. **One coverage line**, for example (run 1's numbers; 914 and 98 are estimates from the server
   copy tables): "The harness's own log: 1,261 calls by 43 threads. Recorded:
   914 requests, 98 refused by the provider. Harness calls with no recording: 473, from 15:43 to 16:54
   (71 min). Recording off: 0 times." When no harness reader exists for the connector: "The harness's
   own log is not read for this harness", never "everything recorded".
4. The timeline (10-minute rows), the thread table (parent, first and last time, harness calls, wire
   calls, errors, models from PRD-09), gaps by reason with lost bytes and times, hosts (decrypted hosts
   with request counts, tunnelled listed hosts with their reason, long tunnels to unlisted hosts).
5. The check results, one line each.

It runs on the organizer's machine after `fetch-run.sh`, never in the Worker (128 MiB window, PRD-01
section 15). It prints counts, times, host names and thread ids only.

### 6. What of `crates/goalcup-verifier` stays and goes

The whole crate is deleted by PRD-13 after this PRD's tests pass. Nothing is kept as code except what
is copied below.

| Part | Lines (src / tests) | Fate |
|---|---|---|
| `bin/goalcup-verify.rs` | 326 / 0 | Copy `create_output_file` (`:264-294`) and the parse rule (`:296-310`). Cut the three flag modes and `--dev-key` (`:44-110`) |
| `lib.rs` | 53 / 0 | Cut: `SERVER_PUBLIC_KEYS` (`:31-32`) and `VerifyOptions` (`:36-53`) go with signing |
| `capture_analysis/reader.rs` | 1,889 + 181 inline / 3,737 (`tests/it/m2_capture_integrity.rs`) | Keep the ideas, rewrite for GCR1: one streaming pass, bounded distinct finding codes (`:442-476`), first failure, exit mapping (`:269-281`), the hole rule (`:1726-1733`, now invariant 4). Cut: receipt and ack checks (`:620-665`, `:925-975`), baseline and seal manifests and Git reads (`:985-1110`), launch goal rules (`:1115-1125`, `:1818-1889`), producer admission and recovery (`:1378-1660`), the caps (`:78`, `:1384`, `:1566`, `:1694`) |
| `capture_analysis/lineage.rs` | 101 + 41 / 0 | Cut. Producers and recoveries are gone; boot order replaces them |
| Decoders: `anthropic.rs` 1,638, `openai.rs` 664, `chat.rs` 1,080, `thread.rs` 383, `native/*` 1,279, `wire.rs` 1,339, `tracker.rs` 600 | 6,983 / about 11,000 | PRD-09 copies only the model, usage, error and thread parts (for example `openai.rs:470-486, 545-558`; `native/codex.rs:125-128, 286-308, 418-427`). Everything else cut |
| Framework: `api.rs` 313, `registry.rs` 213, `session.rs` 940, `operations.rs` 348 | 1,814 / 1,253 | Cut. `cross_leg` is replaced by `wire_keeps_up`, the label by the judge view |
| `report.rs` | 946 / 2,456 | Cut |
| `stats.rs` | 1,115 / 873 | Cut. The one-error-per-call rule moves to PRD-09 |
| `tests/it/v5_corpus.rs` and `tests/fixtures/v5-corpus/` | 0 / 718 plus 912 fixture lines | Cut |
| Other tests and fixtures | rest of 20,724 / 2,182 | Cut. PRD-17 reads `m2_capture_integrity.rs` for lessons; PRD-09 decides which `m2-*` and `connectors` fixtures to copy |

Couplings that go with the crate: `crates/goalcup-cli/Cargo.toml:33` and the doctor decoder check
(`doctor.rs:246-316`) go with `goalcup-cli` (PRD-13); `scripts/matrix.sh`, `research-replay.py` and
`linux-harness-smoke.sh` callers go to PRD-13 and PRD-14; the pad replaces them.

## Files it owns

Create (all new, in the PRD-00 tree):

- `crates/goalcup-verify/Cargo.toml`: every dependency declared once, for PRD-09 too (`goalcup-format`,
  `serde`, `serde_json`, `sha2`, `clap`, `regex`, `base64`). Nothing async, no TLS.
- `crates/goalcup-verify/src/main.rs`: arguments, dispatch (including `usage`, which calls PRD-09's
  `usage::run`), exit codes, safe output files.
- `src/lib.rs` (module list), `src/input.rs` (entry, run, fetched and live inputs; part and chunk
  order), `src/integrity.rs` (`inv1` to `inv9`, boots), `src/honesty.rs` (`gap_has_end`,
  `streams_closed`, `seen_matches_written`, `recorder_alive`, `listed_hosts_decrypted`, `conn_bytes`),
  `src/legs.rs` (`wire_keeps_up`, `threads_match`, `subagents_present`, `provider_errors`, `timeline`),
  `src/server.rs`, `src/secrets.rs`, `src/judge.rs`, `src/report.rs` (S6 and text).
- `crates/goalcup-verify/tests/check_*.rs`, `tests/server_*.rs`, `tests/secrets_*.rs`,
  `tests/judge_*.rs`, `tests/common/` (record builders over `goalcup_format::PartWriter`, the run 1
  shape builder).

Change: none outside the crate. Delete: none (PRD-13 deletes the old crate).

Must not touch: `crates/goalcup-verify/src/usage/` and `tests/usage_*.rs` (PRD-09),
`crates/goalcup-format/` (PRD-01), `crates/goalcup/` (recorder PRDs), `server/` and
`scripts/fetch-run.sh` (PRD-07), `scripts/pad/` (PRD-12), `docs/RULES.md` and
`docs/CHEAT-CHECKLIST.md` (PRD-15), the old crates (PRD-13).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 | F1: `read` over parts and chunks with an open end, `dump`, record types, link check, `read_index` for `--live`; the run 1 converter (acceptance 4) | Used here |
| PRD-02 | GAP tail keys `guard`, `limit`, `cause`, `prev`, `exits`, `harness`; exit codes 70, 74, 75 (shown in the judge view); COUNT every 60 s; a `test-hooks` silent-drop hook for proof R5 (ask 5) | Used here |
| PRD-03 | `conn` meta `listed`, `mode`, `reason`; END tails `up`, `down`, `wire`; `note` `long_tunnel` | Used here |
| PRD-04 | Native meta `path`, `gen`, `from`, `fid`; END `source_changed` with no GAP is allowed (PRD-04 ask 3); GAP tail `unread_at_least` | Used here |
| PRD-05 | The rule table as data the verifier compiles without the left guard (ask 1 of PRD-00); rule names in `mask_rules` | Used here |
| PRD-06 | `upload.json` `keep`; `GOALCUP_KEEP_PARTS=1` for `server_equals_local` | Used here |
| PRD-07 | Fetched layout: per entry, chunks in offset order with their keys and SHA-256, plus `entry.json` with the database row read in a `READ ONLY` transaction (`model_requests`, `upload_offset`, `final_seq`, final head, `recording`, sealed time); the final heartbeat's `seq` and hash | Used here (ask 3) |
| PRD-09 | Seam U1 (ask 2): a record sink `usage::Calls::push(&Record)` then `finish() -> CallTable`. Rows: leg (`wire` or `native`), entry, `sid` or `path`, thread id, parent thread id, wall time, kind (`call`, `warmup`, `interrupt`, `other`), outcome (`ok`, `error{code, status}`, `unknown`), model. Plus: `readable` per connector (is the harness log read at all) and the children each parent names | Used here |
| PRD-10 | `goalcup seal` prints a `goalcup-verify check` line only when local parts are whole; otherwise it names the server copy. The participant's counts are the ones `counts_match_seal` recounts | Used there |
| PRD-11 | Connector name and `hosts` lists in the `start` event decide the harness reader and `listed_hosts_decrypted` | Used here |
| PRD-12 | S6 report; the `--harness-files` manifest and `--known` file the pad writes; the pad's four checks map to: wire against log = `wire_keeps_up` + `threads_match`; every subagent = `subagents_present`; no secret = `secrets`; server whole = `server_*` | Used there |
| PRD-13 | Deletes `crates/goalcup-verifier/` after this PRD's tests pass | Done there |
| PRD-14 | `cargo test -p goalcup-verify` in `gate fast`; the release ships `goalcup-verify` | Used there |
| PRD-15 | RULES "What a label is worth" and the CHEAT-CHECKLIST cross-leg text rewritten for the judge view and the four labels | Written there |
| PRD-16 | Watchers run `check --live` every 5 minutes and `judge` after each run; a `fail` goes in the night log | Used there |
| PRD-17 | Lesson cases are records fed through `check`; each must reach exit 0 or a named result, never a crash | Used there |

## What is cut

| Path | Lines (src / tests) | Tests that go |
|---|---|---|
| `crates/goalcup-verifier/` (whole crate, section 6) | 13,488 / 20,724, plus 2,182 fixture lines | All of `tests/` |
| Flags `--capture-integrity`, `--research-json`, `--stats`, `--dev-key` | in the above | `v5_corpus.rs` exit-code cases |
| The research report NDJSON format (`docs/research/m2-report-format.md` is PRD-15's to retire) | in the above | `m2_research_report.rs`, byte goldens in `tests/fixtures/m2-report-bytes/` |

New code: about 1,500 src lines (input 120, integrity 350, honesty 200, legs 250, server 130,
secrets 200, judge 150, report and main 100) and about 1,200 test lines. PRD-00 budgets 1,200 src;
the secret scan and the judge view were not in that figure (ask 4).

## Acceptance

### Real-run proofs (first)

| # | Proof | Pass |
|---|---|---|
| R1 | **Run 1 fails.** Run 1's server copy, read-only, rewritten to GCR1 by PRD-01's test converter (ask of PRD-01 7), then `check --server` and `judge` | Exit 2. `gap_has_end` fails on 23 gaps. `streams_closed` fails. `wire_keeps_up` fails with the wire's last call at 15:43:1xZ and about 473 harness calls after it. `threads_match` fails on the root thread and on 15 threads with no wire call. `subagents_present` fails on 11 children. `provider_errors.wire` holds 97 to 99 `rate_limit_exceeded`, not 0 and not about 196. `server_chunks_whole` passes (20 chunks). `server_counts_match` fails (1,766 stored against the written count). The judge view's coverage line names 15:43 to 16:54 |
| R2 | **A clean run passes.** Claude Code, direct key, one subagent (PRD-12) | Exit 0. Every check `pass` or `skip`. `threads_match` covers the main thread and the subagent. `server_equals_local` passes. `secrets` finds 0 known values |
| R3 | **A killed recorder is named, not failed.** Codex kill test (PRD-01 acceptance 3) | Exit 1. Two `recorder_down` gaps. Every harness call missing from the wire falls inside them, so `wire_keeps_up` and `threads_match` are `named`. No `fail` |
| R4 | **The dodger case is honest.** Codex with its model host marked `pinned` on the pad | Exit 1 or 2, never 0. Every connection to that host is a tunnel with GAP `pinned`; `wire_keeps_up` is `named` by them; the judge label is `native_only` |
| R5 | **The 1.0 shape of run 1 is caught.** A `test-hooks` build that silently stops offering wire copies after 10 minutes (no GAP), Codex with three subagents for 20 minutes | Exit 2. `wire_keeps_up` and `threads_match` fail within the first `check --live` after the 10-minute mark plus 2 minutes; `seen_matches_written` or `conn_bytes` names the streams |
| R6 | **Load.** Codex with 20 or more subagents for 30 minutes (PRD-03 proof 10) | `threads_match` passes or is `named` for every thread; `provider_errors` shows any 429s; `check` reads the entry in under 1 minute on the sandbox |
| R7 | **Secrets.** The pad's planted `.env` and prompt (PRD-05 A1), all three cases | 0 known values in local parts and server copy; every planted shape has a MASK; glued keys are reported |

### Automated tests that guard them

- `check_invariants.rs`: a valid entry built with `PartWriter` exits 0; one mutation per invariant
  (`inv1` to `inv9`), a torn tail and an unknown record kind give the right code and exit.
- `check_run1_shape.rs`: a small record shaped like run 1 (root and four Codex-shaped threads, wire
  stops at minute 49 while rollouts grow to minute 120, six rate-limit errors sent both as an `error`
  event and a `response.failed`, three GAPs with start equal to end and unknown loss, two streams with
  no END in a sealed entry, one child with neither leg). Pass: exactly those checks fail, exit 2,
  `provider_errors` reads 6. The same record with the loss removed exits 0, so the checks do not fire
  on a clean run.
- `check_named.rs`: the clean record with one `recorder_down` gap that covers the missing calls: exit 1,
  results `named`.
- `server_compare.rs`: fetched chunks equal to local parts pass; one flipped byte, a missing middle
  chunk and a wrong database `model_requests` each fail.
- `secrets_scan.rs`: a known value split across two DATA records, in base64 at each alignment,
  percent-encoded and spread over three SSE deltas is found; the same value as `*` is not.
- `judge_view.rs`: a golden of the clean record's page.

## Risks and blind spots

- **False alarms in `wire_keeps_up` and `threads_match`.** A harness may write a call twice, or a
  harness's log format may change between versions (Claude Code says it does,
  `docs/plans/2026-10-30-audit/D-harnesses.md:276-278`). Then the check fails a good run. Guard: the
  rule only fires on harness above wire; the tolerance is max(3, 2%); R2 and R6 must pass on real runs
  before the pad trusts it. If PRD-09 reads zero harness calls while the wire has calls, the result is
  `skip` with "could not read the harness's own log", never `pass`.
- **The check depends on PRD-09's readers.** Harnesses with no reader (Hermes until its database is
  read, Amp) get `skip` for the leg checks. Then a silent loss is caught only by
  `seen_matches_written`, `conn_bytes` and `recorder_alive`. The judge view says the log was not read.
- **A loss before a stream opens is invisible.** If the proxy drops a whole connection without a
  `conn` record, only the harness's log can show it. For a harness with no session files, nothing can.
- **`conn_bytes` overhead is unknown** (HTTP/2 framing, WebSocket frame headers, deflate). It stays
  `warn` until measured.
- **Thread hints are per harness.** A harness whose wire requests carry no thread id gets run-level
  `wire_keeps_up` only; `threads_match` is `skip`.
- **The verifier trusts the record it reads.** Signing is cut, so it proves the server copy whole and
  consistent, not that the participant's machine was honest. The server's `received_at` bounds time.
- **Run 1 proof needs the converter and read-only access.** If either is missing tonight, R1 runs only
  as `check_run1_shape.rs`, and the night log says so.
- **Cost on a busy week.** About 140 GB raw (PRD-01 section 14) is one decode and one SHA-256 pass,
  estimated 10 to 30 minutes on 4 vCPU (unverified). `check --live` reads the whole entry each time.
  That is fine for 2 hours; for a week the watchers run it hourly.
- **Judges lose the decoded research report.** PRD-09's usage JSON and the judge view replace it.

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Harness above wire tolerance | max(3 calls, 2%), with a 2-minute lag | PRD-16 after R2 and R6 |
| Wire ends before native | 10 minutes | PRD-16 |
| COUNT silence | 180 s | PRD-02 if COUNT cadence changes |
| `conn_bytes` | `warn` at 10% and 1 MiB | PRD-16 after three pad runs |
| Biggest MASK span | 16 KiB | PRD-05 |
| Secret scan join overlap | 4 KiB | PRD-05 if a rule matches longer values |
| Timeline bucket | 10 minutes | anyone |

## Builder notes

Order of work:

1. `main.rs`, `input.rs` and `integrity.rs` over PRD-01's reader, with `check_invariants.rs`. Publish S6
   with real `inv*` results and `skip` for the rest by H+3: the phase 1a watchers need it.
2. `honesty.rs` and `report.rs` counts; `counts_match_seal`.
3. `server.rs` as soon as PRD-07 fixes the fetched layout.
4. `legs.rs` against a stub `CallTable`; switch to PRD-09's sink when U1 lands. Build
   `check_run1_shape.rs` with Codex-shaped bodies so PRD-09's real reader can run on it.
5. `secrets.rs`, then `judge.rs`.

Traps from the audit:

- Do not compare raw wire attempts with harness calls: the wire counts retries and warm-ups (appendix
  M section 5.6 lesson 1). Compare non-error wire calls, and only flag harness above wire.
- Do not count an `error` event and a `response.failed` for one call twice (`openai.rs:475-486,
  545-558`); run 1 would read about 196.
- Take errors from the wire leg always; today's verifier took them from the token leg
  (`stats.rs:949-956`), which hid run 1's 429s.
- Never turn a cap into a verdict (`reader.rs:1694`): a large run is not an invalid run.
- Never decode a chunk alone: a chunk continues its part's frame (PRD-01 section 12). Concatenate, then
  decode.
- Never print a byte of a DATA record in `check`, `secrets` or `judge`; only `dump` prints payload, and
  only on request.

How a watcher proves it in a real run: after each pad run, `goalcup-verify check --json` on the entry
and `goalcup-verify check --server <fetched> --local <entry> --known <keys>`; the pad table reads
`result` and the four mapped checks. During the 2-hour run, `check --live` every 5 minutes: a `fail`
on `wire_keeps_up` or `recorder_alive` is the alarm run 1 never raised. Once a night, R5.

## Asks of PRD-00 and PRD-01

PRD-01:

1. **Invariant 4 from `meta.from`** (as PRD-02 ask 3 and PRD-04 ask 2): per native path, DATA is
   contiguous from `from` across boots.
2. **A GAP has an extent.** Add to section 13: every GAP has `end_mono > start_mono`, or `lost > 0`.
   A GAP with start equal to end and unknown loss is invalid. That is exactly run 1's gap, and the
   format should forbid it, not only the verifier.
3. **Stream closure as an invariant.** Section 7 says unended streams of a dead boot are "read as cut".
   Make it invariant 10: in a sealed entry, every stream has an END or belongs to a boot followed by a
   `recorder_down` gap; streams still open at the seal end `cut_at_seal`.
4. **`seen` as an invariant.** Say that `seen` counts decoded bytes in the DATA offset space (PRD-03
   says so) and add invariant 11: `seen` equals the last written offset plus known lost bytes.
5. **COUNT cadence the verifier can check.** Section 9 says "every 60 s"; state the bound a reader may
   enforce (180 s while the boot lives).
6. **Plaintext bytes on a decrypted `conn`.** Say whether END tail `up` and `down` are TLS ciphertext or
   plaintext. Add `plain_up` and `plain_down` for `decrypt` mode, so `conn_bytes` can compare.
7. **The run 1 converter must keep run 1's faults.** In `crates/goalcup-format/tests/tools/`: a v5 gap
   becomes a GAP with start equal to end and `lost` 0; a v5 stream with no End gets no END; producers
   and recoveries leave no `recorder_down`; COUNT events are synthesized every 60 s by recount, so
   only the honesty checks fail; headers, bodies and rollouts stay as DATA; v5 `local_finish` becomes
   the seal event.
8. **GAP reason numbers.** 12 is asked for twice (`mask_failed` by PRD-02, `upstream_untrusted` by
   PRD-03). Fix both numbers before H+1. The verifier shows reserved reasons 8 and 9, if ever seen, as
   `other(n)` and `warn`.

PRD-00:

1. **Seam S9: the secret shapes as data.** PRD-05's rule table in a plain file under `src/mask/` that
   the verifier includes at build time and compiles without the guard. Otherwise the verifier must link
   the recorder crate or keep a second copy that drifts.
2. **Seam U1 between PRD-09 and PRD-08** (the call table above), stable by H+4 like S7. Add it to the
   seams table.
3. **Seam S10: the fetched layout** (PRD-07), including `entry.json` with the database row and the final
   heartbeat's `seq` and hash, stable by H+3. Without it `server_counts_match` cannot run.
4. **Size budget:** verifier about 1,500 src, not 1,200 (secret scan and judge view).
5. **A silent-drop test hook** in PRD-02 or PRD-03 under PRD-14's `test-hooks` feature, for proof R5.
   It is the only way to prove on a real run that the checks catch run 1's fault in the new shape.
6. **Ownership note:** PRD-08 owns `crates/goalcup-verify/Cargo.toml` and `src/main.rs` dispatch;
   PRD-09 owns `src/usage/` and exposes `usage::run(args) -> ExitCode` and U1.
