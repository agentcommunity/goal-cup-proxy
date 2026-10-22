# M2 research report format (research-report schema 4)

`goalcup-verify --research-json TRACE --output REPORT.ndjson` writes a private, streaming report
of one protocol-v5 capture trace. The library function
`goalcup_verifier::write_research_report(trace, options, output)` is the same writer: the same
trace and options give the same bytes from both.

The report is private. It holds decoded, sanitized facts and short sanitized goal excerpts, and
nothing publishes it. It never modifies the trace. It is not a verdict on the task and not proof
that no one helped.

## Invocation

- `--output` names a new file ending in `.ndjson`. An existing file is refused, and so is any
  path inside the trace directory. The file is created with mode `0600`.
- Stdout stays empty. Stderr carries the development-flag notices and one summary line.
- `--research-json` cannot be combined with a trace directory, `--capture-integrity` or `--json`.
- Every refused invocation exits 3 before anything is written.

Use an immutable, sealed trace or a complete retrieved copy for repeatable analysis. Rotated chain
parts are opened lazily while the reader walks the trace: a part already open remains readable if
upload pruning later unlinks it, but a subsequently enumerated part that has disappeared causes an
I/O failure and no usable report. Missing parts are not skipped or re-enumerated. A trace whose
initial parts were already pruned remains incomplete (`chain_pruned_locally`). The reader does not
lock the trace against concurrent upload pruning.
- `--dev-key HEX` keeps its `--capture-integrity` meaning, and the summary records it under
  `options`.

## Lines

The file is zero or more observation lines followed by exactly one summary line. **A file whose
last line is not a summary line is incomplete**: the trace could not be read, or writing failed
part way. That run exits 3 and says so on stderr.

### Observation line

```json
{"type":"observation","decoder":"openai-responses","observation":{...}}
```

`observation` is the decoder's own `Observation`, unchanged: `source` (the stream and the
retained segment parts it was decoded from), `observed` (the capture-time interval of those
segments), `kind`, `supplied_id` and `fields`. Lines appear in the order decoders emit them while
validated capture records are fed through one `DecodeSession`, front to back. The kinds and fields
each decoder emits are described in that decoder's module documentation under
`crates/goalcup-verifier/src/capture_analysis/`.

The report reader validates `trace/chain.jsonl` through `scan_capture_trace`, then gives each
validated `CaptureRecord` to `DecodeSession`. `DecoderRegistry` supplies the candidates and the
session routes accepted streams to `DecoderV1` instances. Decoders never open `chain.jsonl` or a
trace directory directly.

The research-report kind vocabulary (schemas 1 and 2) is closed at these ten values. Code emits them through
`capture_analysis::api::kinds`; `reasoning` is an item or field value, not an observation kind.

| Kind | Emitters | Meaning |
|---|---|---|
| `model_request` | wire decoders | One provider request and its supplied request fields. |
| `model` | native and wire decoders | A model identity supplied by the source. |
| `model_response` | wire decoders | Source-qualified response content or one response content item; an application thread snapshot does not establish a direct provider response. |
| `tool_call` | native and wire decoders | One tool invocation supplied by the source. |
| `tool_result` | native and wire decoders | One returned or replayed tool result. |
| `usage` | native and wire decoders | One source-supplied usage snapshot. |
| `outcome` | native and wire decoders | A provider or session outcome, never a task-success verdict. |
| `error` | native and wire decoders | A provider or session error, with only safe supplied fields. |
| `unknown` | native and wire decoders | One emitted, source-referenced unknown condition; never a decoded fact. Spanless findings have no observation line. |
| `human_message` | native decoders only | A message positively marked as human-authored by the source. |

### Optional native source-session context

A native decoder may add decoder-versioned `fields.native_context` to parsed observations.
This is source-session provenance, not per-record authorship or proof of fresh activity. The
shared convention is `session_id` (the source's own session), optional `parent_session_id`,
`role`, optional `launch_source`, and `header_source` (a `SourceRef` to the supplying header).
`identity_field` and `parent_field` identify the supplied field paths. Missing, conflicting or
overlong identities are omitted rather than guessed. This convention can be adopted by other
native formats; their decoders document their own source mappings and versions.

`codex-native` version `3` fixes authority and context from its first `session_meta`. A later
copied parent header cannot replace the child or turn child user-role records into typed human
input. Own identity comes from `payload.id`; `payload.session_id` can still name the parent.
IDs are nonempty strings of at most 256 UTF-8 bytes. Parent IDs must agree across the supplied
nested thread-spawn, `parent_thread_id`, and `forked_from_id` fields. `role` is `interactive`,
`headless`, `subagent`, `fork`, or `unknown`. Child/fork context includes
`history_scope: "may_include_inherited"`; it does not classify each occurrence as fresh or copied.
Tool/model/history occurrences remain separate, including repeated supplied IDs across streams.

Ordinary CLI/vscode input still requires the existing positive message marker and does not
require a session ID. A malformed/ambiguous first header cannot be repaired into human authority
by a later CLI-looking header. Lost/unreadable context before a first header prevents promotion;
loss after established identity retains that identity alongside the existing explicit loss.
Unsupported or ambiguous user records remain `unknown`; this neither discards raw evidence nor
changes capture integrity, the ten kinds, Decoder API version 1, or report schema 4. Decoder
identity/version and measured-source qualifications remain in `analysis.decoders`.

Version `3` adds: an assistant `response_item.message` is a `model_response` (`role`, `phase`,
`content_types`; no text). An `event_msg.item_completed` `CollabAgentToolCall` item is a
`tool_call` (`name` such as `spawn_agent`, `status`, `model`, `reasoning_effort`,
`receiver_thread_ids`); a child's stream carries the same id as `native_context.session_id`, which
attributes its records to that call. The other completed items (`AgentMessage`, `UserMessage`,
`Reasoning`, `CommandExecution`, `FileChange`) repeat a `response_item` of the same stream and are
`unknown` with `reason: "duplicate_view"`.

### Summary line

| Field | Meaning |
|---|---|
| `type`, `schema_version` | `"summary"`, `4`. Schema 2 added `analysis.models`, `analysis.models_omitted` and `analysis.cross_leg` to schema 1. Schema 3 added `delivery.records_after_deadline`, `delivery.last_seq_at_or_before_deadline` and `delivery.last_ack_at`. Schema 4 added `analysis.undecoded` and `analysis.undecoded_omitted`, and dropped `options.dev_mode` (always `false`; a dev or unpinned key shows in `key_source`, `options.extra_server_keys` and the `server_key_unpinned` finding). Every other key keeps its meaning and observation lines are unchanged. |
| `exit_code` | The verifier/report exit code (below), not the harness run's exit code. |
| `verifier_version` | The verifier build that wrote the report. |
| `options` | The effective options: `allow_descriptor_key`, `extra_server_keys`. |
| `key_source` | Which key checked the receipt and ACK signatures: `pinned`, `supplied` or `none`. This build pins the production ingest key, so a production trace gives `pinned`. A key from `--dev-key` gives `supplied`. |
| `input` | The validated chain prefix the report covers: `validated_head` (the link hash of its last record), `validated_records`, `validated_bytes`. |
| `run.launch` | The first `Launch` record as retained: profile, profile digest, harness version, recorder revision, continuation and the goal stream it names, and, when the daemon wrote them (contract S1), `run_public_key` and `connector_dir_sha256`. Both are additive and absent from a trace written before S1, so a consumer treats a missing key as unknown rather than as an error. `null` when there is none. |
| `run.connector` | Present only when `run.launch.connector_dir_sha256` is present: `Pinned` when the launch profile and digest match this build's connector-directory pins, otherwise `Dev/unpinned`. This presentation does not change capture integrity. It is absent for an old/unbound launch or when there is no launch. |
| `run.terminal` | The reader's verified terminal: `local_finish` (the signed finalization was removed on 21 Oct, D54), its `seq` and `hash`, the typed `termination` outcome exactly as recorded, and `observed` (the local finish capture time; `null` for a signed finalization, which carries wall time only). `null` when no terminal was verified. |
| `capture` | The reader's integrity result: `integrity` (`valid`, `incomplete` or `invalid`), `sealed`, `coverage_incomplete`, `torn_tail`, `gap_count`, `findings`, and `record_counts`, a count of every validated capture record by kind (`open.<source>`, `segment`, `end.<reason>`, `gap.<reason>`, `recovery.<reason>`, `connection.open`, `connection.upgraded_web_socket`, `connection.close.<outcome>`, `producer_closed`, `launch`, `assistance`, `local_finish`). |
| `delivery` | `anchor` (`local_only` or `signed`), `verified_ack` and `receipt_clock` exactly as the reader verified them, and `state`: `local_only` (no verified receipt or ACK), `none_acknowledged` (a verified receipt, no verified ACK), `partial` (a verified ACK behind the terminal, or no terminal), `through_terminal` (a verified ACK at or past the terminal). `records_after_deadline` and `last_seq_at_or_before_deadline` are the reader's counts of validated records stamped after, and the last seq stamped at or before, the receipt's `deadline_at`, by the participant's own machine clock (a hint; `null` without a verified, head-bound receipt). `last_ack_at` is the `acked_at` of the last verified ACK, signed by the server (the trusted time; `null` without one). |
| `assistance` | `explicit_records` (the chain's `Assistance` records), `positive_native_markers` (the `human_message` observations written) and `evidence`: `observed` or `none_observed`. |
| `analysis.state` | `available` (decoded observations written, no decoder degraded), `partial` (decoded observations written, some decoder degraded) or `unavailable` (no decoded observation written; source-referenced `unknown` observations may still be written). `KindTally` excludes `unknown` lines from decoded state. A capture-only stream can coexist with decoded streams; only the absence of a decoded fact makes analysis unavailable, while other decoded streams can make it available or partial. |
| `analysis.decoders` | One entry per decoder, in the order they ran: `decoder`, its own `summary` (`decoder_id`, `decoder_version`, `observations`, `unknown_records`, `incomplete`; `null` if it failed), `written`, `over_budget`, `failure` (`{"phase","error"}` or `null`; `phase` is `create`, `accepts`, `push` or `finish`, `error` a short token such as an I/O error kind, or `panicked`), `measured` (the real traffic the decoder was checked against) and `synthetic_only` (the forms it decodes that only synthetic tests have seen; a judge reads a non-empty list as "never checked on real traffic", for example an unmeasured error form). `unknown_records` counts both emitted source-referenced `unknown` observations and spanless findings counted only in the decoder summary, so it can exceed written `unknown` lines. |
| `analysis.undecoded` | The capture-only streams: the streams no decoder accepted, recorded and not read. One `{"source","hint","streams","bytes","bytes_incomplete"}` per distinct source and `protocol_hint` (`hint` is `null` when the stream had none, and at most its first 64 bytes, the hint grammar's bound; a longer hint is cut and the group adds `"hint_truncated": true`, otherwise that key is absent), in first-seen order, at most 64 groups. `bytes` counts the streams' segment source bytes; `bytes_incomplete` is `true` when too many were live at once to count them all. The goal stream is never listed. A misspelled hint in a new connector shows here. |
| `analysis.undecoded_omitted` | Capture-only streams of groups past the bound. |
| `analysis.models` | What the run used, as its sources state it: one `{"decoder","model","count"}` per model identity a decoder's kept `model` observations name (`fields.model`), in decoder then name order, at most 16 names of at most 128 bytes per decoder. A native decoder's names come from the harness's session file, a wire decoder's from the recorded requests and responses. |
| `analysis.models_omitted` | Per decoder that dropped any, how many `model` observations' names were not kept (past the bound, too long or empty). `{}` when none. |
| `analysis.cross_leg` | `native_model_activity` (kept native-stream `model` observations that are not a switch, `tool_call` and `usage`), `wire_streams` (wire streams the chain opened, decoded or not), `wire_model_requests` (kept wire-stream `model_request` observations), and `state`: `consistent` (native activity and decoded requests), `native_activity_without_wire_streams` (the session file names model work and the recording proxy recorded no wire stream at all: a harness that bypassed the recording proxy, or a fake), or `unchecked` (no native model activity to compare, or wire streams whose requests were not decoded, such as a compressed or oversized body or a degraded decoder). Each leg is summed across its decoders; only zero against non-zero is judged. A note for judges: it never changes `exit_code`. Builds are recorded, not pinned (D53), so this is the check that catches a cheap wrapper. |
| `references` | Operational facts that have no segment, in chain order: each `Gap`, `Recovery` and `Assistance` record, every `Launch` after the first, and one goal reference per goal stream. |
| `references_omitted` | How many references did not fit the bound. They remain in the chain. |

A goal reference is `{"kind":"goal","stream","first_part","last_part","source_bytes","end","excerpt","excerpt_truncated"}`:
the retained parts of the goal stream (`null` when it carried no segment), the source length it
covers, how it ended (`null` if it never did), and at most 1 KiB of its sanitized bytes, cut on a
character boundary. The whole goal stays in the chain.

A decoder is degraded when it failed (it stops, and the others continue), when one of its
observations was over the line bound (that observation is counted in `over_budget` and not
written), or when its own summary says `incomplete`.

Serialized report keys are the research-report schema contract, not a promise that Rust uses the
same spelling.
Serde renames and typed Rust enums may deliberately use idiomatic Rust identifiers while writing
the pinned lower-case or snake-case JSON values. Consumers compare the serialized keys and values,
not Rust type or variant names. Any Rust field addition that would enter research-report JSON
requires a research-report schema bump or an explicit projection layer; Rust's
`#[non_exhaustive]` affects matching and construction, not serialized compatibility.

## Exit codes

- **0**: the trace is complete and valid, and no decoder is degraded. `local_only` anchoring alone
  never lowers this: a complete local trace with no server, receipt or ACK exits 0.
- **1**: the trace is valid but incomplete or unsealed, or a decoder is degraded.
- **2**: invalid evidence, including a torn tail. The observations of the validated prefix are
  still written, and the summary says `invalid`.
- **3**: no usable report: a refused invocation, an unreadable or non-v5 trace, a failed write.

A capture-only stream alone is not a failure. It exists when the session records no accepting
decoder for the full `StreamOpen`; an isolated factory or `accepts` failure can also leave no
selector. The registry supplies candidates and each decoder decides which fields matter. An
unsupported content type may instead be accepted and emitted as `unknown`. The summary lists
capture-only streams under `analysis.undecoded`.
`available` says at least one decoded observation was written and no decoder degraded, not that
every stream had a decoder.

## Bounds

- One observation line: at most 2 MiB, newline included.
- The summary line: at most 64 KiB. Every part of it is bounded and the trace alone stays well
  under it, so a summary over it (possible only with very many supplied keys) is a write
  failure, exit 3.
- References: at most 128 and at most 32 KiB of serialized JSON. They are the first ones in chain
  order; everything after the first that does not fit is counted in `references_omitted`.
- Each decoder bounds its own parser state (8 MiB). The report holds no list of observations.

## Reading it honestly

- **Time.** `observed` is an interval over retained segments in one recorder clock. It is not an
  event instant and not a latency. A source-supplied time appears only as its own field, as
  supplied. The report computes no duration, and a verified receipt clock is reported beside the
  local terminal, never in place of it.
- **Counts are per source.** A native transcript and the wire traffic of the same model call are
  two sources, reported separately. Repeated requests stay separate. No count, usage or total is
  added across observations, decoders or snapshots; cumulative usage stays the snapshot it was.
- **Tool results.** A request that replays history repeats earlier `tool_result` blocks. Each is a
  per-request occurrence keyed by its supplied `tool_use_id`, never a new unique result.
- **Assistance.** `none_observed` means no explicit evidence was recorded. It is not a finding of
  zero assistance. A `human_message` needs a positive marker on the record itself; position in a
  stream and the delivered goal never are one, and unmarked user text is `unknown`. A producer
  recovery is a capture event, not assistance, and its gap stays in the report after it.
- **Provider outcomes are not task outcomes.** An `outcome` or `error` observation is what one
  source said about one response or turn. Nothing in the report states that the task succeeded.
- **Measured and synthetic-only.** `measured` names the real traffic each decoder was checked
  against. `synthetic_only` lists the forms it decodes that were seen only in synthetic tests:
  its handling of such a form follows documented shapes and has not been checked against real
  traffic. Both stay on every decoder entry of the summary, so a judge sees them next to the
  observations they qualify. They are read from the registry entry (`DecoderRegistry::builtin()`).

## Decoders

The report runs the decoders of a `DecoderRegistry`, in registration order. `write_research_report`
constructs a `DecodeSession` over the built-in registry and feeds it the validated records from
`scan_capture_trace`; a decoder does not read the trace directory or `chain.jsonl` itself. The
built-in registry
(`DecoderRegistry::builtin()` in `crates/goalcup-verifier/src/capture_analysis/registry.rs`) lists
`codex-native`, `claude-code-native`, `openai-responses`, `anthropic-messages` and
`openai-chat-completions`, and `amp-thread-jsonrpc`. A new decoder
implements `DecoderV1` (`capture_analysis/api.rs`) and becomes one more `Registered` entry, with
the traffic it was measured on and its synthetic-only forms. The registry supplies every decoder
as a candidate; `accepts` receives the full `StreamOpen`, and that decoder decides which fields
matter. A changed interpretation changes that decoder's version, which every summary records.

Decoder v1's stable public boundary is `DecoderV1`, `StreamOpen`, `DecoderFailure`, `Observation`
(including `SourceRef` and `ObservationTimeRange`), `DecoderSummary`, `ObservationSink`,
`Disposition`, `Registered`, `DecoderFactory`, `RegistryError`, `DecoderRegistry`, the aggregate
`DecodeOutcome`, its per-decoder `DecoderOutcome` element type, and reachable `DecoderFailed`,
`DecoderPhase`, `ObservationCounts`,
`Undecoded`, `Operations`, `Reference`, and `GoalReference` support types, and `api::kinds`.
Public orchestration, report and reader helpers such as `DecodeSession`, `decode_records`,
`JsonLinesSink`, `KindTally`, reader facts, `CaptureIntegrityReport`, `ResearchResult`, and
`research_summary` are usable public types, but they are outside that Decoder v1 promise.
Protocol types reachable through this boundary retain their own protocol-version contract.

`openai-chat-completions` (Hermes/DeepSeek; HTTP request bodies, SSE responses and JSON
responses, no WebSocket) emits the existing kinds; its fields are in its module documentation
(`capture_analysis/chat.rs`). Three behaviours differ from the other wire decoders and are
recorded here because they change a summary:
- A completion is many chunks. Message text and tool calls (merged by their supplied `index`)
  are emitted once, at `[DONE]` or at the end of the stream, with `stream_complete` and, when
  chunks were lost, `unreadable_events`. `model` is emitted once per distinct supplied model value
  within a response; `usage` and `finish_reason` remain emitted per chunk.
- Everything up to `[DONE]` is complete. Non-empty data after it emits one `unknown` observation
  with reason `data_after_done` (`events`, `bytes`); no decoded fact comes from the trailing data.
- An `End` `interrupted` or `decode_error` is a faithful early-end finding and leaves the decoder
  summary `incomplete` false when no bytes were lost (D45). The shared wire framing uses this
  close classification. Provider fields such as `done_seen`, `stream_complete`,
  `message_complete`, or incomplete tool input remain provider detail; gaps, dropped bytes,
  bounds, recovery, and unterminated streams still degrade analysis.

`amp-thread-jsonrpc` version `2` reads only WebSocket client/server messages with that exact
hint. It interprets complete server `message_added.params.message` snapshots, with
`scope: "application_thread"`: assistant text/thinking as `model_response`, assistant
`tool_use` as `tool_call`, and user `tool_result.run.result.output` (or a `run.result` that is
the text itself, as a `Task` subagent's answer is) as the context-visible `tool_result`, with
`subagent_tool_uses` counting a `Task` run's `progress[].tool_uses`. A snapshot whose
`params.parentToolUseId` (or `parentToolCallId`) names a parent `Task` call is a subagent's: every
fact from it carries `parent_tool_use_id` (version 2). Model-facing tool names stay as supplied; executor names do not replace them.
Safe supplied `messageId`, `threadId`, block IDs and block indices identify each occurrence.
Tool input is bounded serialized JSON; text/thinking and tool output use bounded text fields.
A supplied assistant `usage.model` yields `model`; the integer `inputTokens`, `outputTokens`,
`cacheCreationInputTokens`, `cacheReadInputTokens`, `totalInputTokens`, and `maxInputTokens`
remain per-snapshot `usage` fields. Values are not summed or completed from earlier messages.

This thread evidence exposes neither a direct provider request nor the exact hidden system or
inference context. It emits no `model_request`, `human_message`, or provider `outcome`.
Deltas, leases, executor events, user send/echo, settings, RPC results, future forms and literal
ping/pong remain source-referenced `unknown` observations. Batch array items are handled
individually under the existing list bound. There is no cross-message accumulator or global
deduplication: repeated complete snapshots remain per-source occurrences. Loss, redaction,
malformed JSON, capture decode errors and limits cannot establish complete content. The native
`amp-thread-metadata-no-transcript` leg and HTTP streams remain capture-only. Measurement is
two retrieved macOS Amp `0.0.1792209635-g24a9fe` samples, one with `Task` subagents; thinking blocks and complete-message
batch arrays remain synthetic-only. See the [Amp facts](../harnesses/amp-cli.md).

The Chat Completions registry measurement now names the retrieved/replayed Hermes HTTP sample
with three SSE replies and one JSON reply. The harness version was unreported; usage-only chunks,
error bodies and unusual stream endings remain explicitly unmeasured forms. This is a sample
qualification, not coverage of every harness/provider version.

Registering a decoder adds one entry to `analysis.decoders` of every summary, including those of
traces whose streams it does not accept; the pinned report bytes under
`crates/goalcup-verifier/tests/fixtures/m2-report-bytes/` carry that one row.

Library use. `write_research_report` is `write_research_report_with` over the built-in registry:
it scans the trace through `scan_capture_trace` and feeds the validated records to
`DecodeSession`. Library callers that need non-default keys or development mode use
`scan_capture_trace` directly. A host that holds records instead
composes `decode_records` (observations to an `ObservationSink`, here `JsonLinesSink`), the reader's integrity report and
`research_summary` (given the sink's kind tally); the output equals the folder entry's for the
same records. The decode outcome's capture-only streams (`undecoded`) are the summary's
`analysis.undecoded`, so a reader can tell "no decoder" from "a decoder found nothing".
