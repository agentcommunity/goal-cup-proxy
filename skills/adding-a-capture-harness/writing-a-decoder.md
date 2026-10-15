# Writing a capture decoder

Reference for turning a harness's retained wire bytes into verifier observations. Without a
registered decoder a harness's claim level is capture-only.

Contents: where the bytes are · reading them · the seam · shared plumbing · when a fact spans
records · must and must not · the redaction marker · fixtures · tests · oracles · records ·
rulings that closed old questions.

Worked example, written against this guide: `capture_analysis/chat.rs` (`openai-chat-completions`,
Hermes/DeepSeek). Read it beside `openai.rs`; the two differ only where the provider's format
does.

## Where the bytes are

- The run's `trace/chain.jsonl` stores hash-linked v5 envelopes. It is input to the verifier's
  reader, not a file a decoder opens. Capture records are defined in
  `crates/goalcup-protocol/src/capture.rs`: `Open` (its `CaptureMeta` carries `source`,
  `protocol_hint`, `content_type` and the wire identity), `Segment` (`part`, `source_offset`,
  `data_b64`, `audit`), `End` (`reason`), `Gap`, `Recovery`, `ProducerClosed`.
- Sources: `wire_request` / `wire_response` (one HTTP body each), `ws_client` / `ws_server`
  (one WebSocket message per stream), `native` (session files), `goal`.
- Bytes are redacted before hashing and disk (D36). `data_b64` is base64 of the redacted
  bytes; audit spans are absolute source offsets in that stream, not base64 or JSON offsets.
- Segment boundaries are arbitrary (at most `MAX_SEGMENT_BYTES` each): mid-line, mid-JSON,
  mid-SSE event. Never assume one event per segment.
- The proxy (`crates/goalcup-cli/src/proxy/http.rs`) submits **decoded** body bytes: chunked
  framing removed and at most one `gzip`, `x-gzip` or `deflate` coding inflated. Any other
  coding records an `UnsupportedEncoding` gap and no body bytes. A decode failure keeps the
  decoded prefix and ends `DecodeError`. EOF inside a body ends the stream `Interrupted`. No
  header, query or credential is retained. HTTP/2 is not captured.
- The protocol hint comes from the connector: `[[capture.hosts]].protocol_hint` (Hermes:
  `openai-chat-completions` in `connectors/hermes-cli/harness.toml`).
- `CaptureMeta` also carries the HTTP `status` of a response (an error body is told apart by
  it, not by the bytes alone) and a `content_type` that is already the media type without
  parameters (`content_type_essence`: `application/json; charset=utf-8` is recorded as
  `application/json`), so an exact match is the right test.

## Reading them

- The report path calls `capture_analysis/reader.rs` `scan_capture_trace` to validate the trace,
  then feeds each validated record to a `DecodeSession`. The session uses a `DecoderRegistry` to
  route records to `DecoderV1` instances. A decoder receives `StreamOpen` and `CaptureRecord`
  values through that session; it never opens `trace/chain.jsonl` or the trace directory itself.
  A reader callback receives each record after it was validated on its own. That is not a verdict:
  a later break, torn tail or gap can still make the trace invalid or incomplete.
- The decoder never changes the `--capture-integrity` exit. In `--research-json`, a degraded
  decoder (failed, over the line bound, or summary `incomplete`) raises the exit to 1:
  `research_summary()` in `capture_analysis/report.rs` takes the maximum of integrity and degraded.
- Judgement lives in the verifier only; the daemon never decodes (D3).

## The seam

- A new file in `crates/goalcup-verifier/src/capture_analysis/`, declared in its `mod.rs`.
- The contract is `api::DecoderV1` (`DECODER_API_VERSION` 1), the only way to write a decoder:
  `id`, `version`, `accepts` (an explicit deterministic predicate over the full `StreamOpen`),
  `push` per accepted stream record or stream-less record, and `finish` returning `DecoderSummary`.
  Errors are `DecoderFailure` tokens, never record bytes (`DecoderFailure::from_io` keeps only
  `io::ErrorKind` word). `DecoderV1: Send`; one fresh instance processes one decode session. Build
  `Observation`, `SourceRef`, `ObservationTimeRange` and `DecoderSummary` with their `new`
  constructors (they are `#[non_exhaustive]`); the session stamps the summary's id and version
  from the registry entry, so a decoder never names itself.
- Export `DECODER_ID`, `DECODER_VERSION` and `PROTOCOL_HINT` like `openai.rs` and
  `anthropic.rs`, and have `accepts` and the decoder's own first-line guard call one private
  `selects`. The guard is a `debug_assert!` plus early return, not a second selection.
- Register one `Registered::new(|| Box::new(MyDecoder::new()), measured_on, synthetic_only)` in
  `DecoderRegistry::builtin()` in `capture_analysis/registry.rs`: the real traffic it was measured
  on and the forms only synthetic tests exercised. The factory is a closure, so a library host
  can register decoders of its own (`DecoderRegistry::register`); the CLI runs the built-in list.
  There is no dynamic loading. Append to the end of the list (a report lists decoders in this
  order). `DecoderRegistry::builtin()` supplies the candidates; `hints::valid_hint` validates only
  the bounded grammar. `m2_decoder_v1.rs` pins acceptance behavior.
- `DecodeSession` asks every registered decoder whether it accepts the complete `StreamOpen`. It
  delivers accepted stream records only to the decoder instances that accepted that stream.
  Stream-less records are delivered to every live decoder, including decoders that did not accept
  any stream. A decoder's `push` and `finish` therefore process values supplied by the session;
  they do not read a trace path or `chain.jsonl`.
- **Registering a decoder changes every pinned report.** The summary's `analysis.decoders` lists
  every registered decoder, so one more entry adds one row to each of
  `tests/fixtures/m2-report-bytes/*.ndjson` (regenerate with `REGENERATE_REPORT_BYTES=1 cargo test
  --offline -p goalcup-verifier --test it m2_research_report::report_bytes_are_pinned`) and to
  `goalcup-cli/tests/fixtures/harness-golden/decoder-selection.json` (regenerate with
  `GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test --offline -p goalcup-verifier --test it harness_contract_golden::`).
  Check the diffs are only the new row (and, in the golden, the probes your hint now reads).
  Also edit the id list in `m2_decoder_v1.rs` (registry order) and `DECODERS` in
  `m2_research_report.rs`, whose mixed trace must say whether your decoder saw a stream. The v5
  reader corpus pins only the reader's record counts, which no decoder changes.
- Capture-only means the session records no accepting decoder; retained bytes and `undecoded`
  are not errors.
- The session routes at most `MAX_OPEN_STREAMS` live streams to a decoder; an `Open` past that
  fails the decoder (`too_many_open_streams`) rather than truncating it. A reader-validated trace
  cannot reach it. A `Recovery` ends the recovered producer's streams (every wire stream if that
  producer was never seen, as in the reader); a `ProducerClosed` ends its producer's.
- A panic or `DecoderFailure` in any call stops that decoder only (its summary becomes a
  `failure`); a failed decoder is dropped inside the isolation and never called again. A
  server that sets `panic = "abort"` loses the isolation. A panic message must not carry record
  bytes: decoder panics are silent only through `session::quiet_panic_hook`, installed over the
  hook current at the first session. A host hook installed later replaces it; wrap it with
  `set_hook(quiet_panic_hook(take_hook()))`.
- Kinds and summary schema: the research report format (`docs/research/m2-report-format.md`, git history at `6494030`).
  Emit only `api::kinds` constants: `model_request`, `model`, `model_response`, `tool_call`,
  `tool_result`, `usage`, `outcome`, `error`, `unknown`, `human_message`. Reuse the existing field names
  (`scope`, `call_id`, `finish_reason`, `withheld`, `unrecognized_fields`, the `*_bytes` and
  `*_truncated` pair). Document your decoder's fields in its own module docs, as `openai.rs` and
  `chat.rs` do; there is no cross-decoder field table.
- Do not write a second SSE framer: see "Shared plumbing" below.

## Shared plumbing

All three built-in wire decoders share these `capture_analysis/wire.rs` transport mechanics:

- `Span`, `SegmentView` and retained-segment projection: which segments contributed to a record
  (`touch` per byte, `merge` to extend a span over later records), with the redaction audit kinds.
- `Sse` and `SseItem`: the framer. `feed` calls your closure with `Data` (a complete event's
  optional `event` name and joined `data` field), `Gap` or `OverBudget`; it frames lines into
  events and nothing else.
  `finish` releases the buffer at the end of the body and says why a last event was never
  decoded. CRLF, comments, multi-line `data`, resync after a gap and the event bound live here.
- Aggregate buffer accounting, `append_bounded`/`release_buffer`, and `classify_close`: one bounded
  path and one faithful Complete/Interrupted/DecodeError/ProducerLost/Unterminated classification.

Responses and Chat additionally share `Origin`, `Out`, `Tally`, `parse_object`, and several
safe-field helpers. Anthropic intentionally retains private totals/output/helpers, its adapter and
message state. Provider payload interpretation stays local; `wire.rs` owns no universal shape.

An emitted `unknown` observation is source-referenced, never a decoded fact, and excluded by
`KindTally` from decoded analysis state. A spanless finding has no line but still increments
`DecoderSummary::unknown_records`, so that count may exceed written `unknown` lines.

## When a fact spans records

Some formats give one fact over many records: a Chat Completions message is many `delta`
chunks, a tool call is merged across chunks by its supplied `index`.

- Read each record on its own as soon as it is complete (a usage chunk, a `finish_reason`, a
  model name need nothing else) and emit those at once.
- Aggregate only what must be (text, tool-call arguments), keyed by a supplied index or id, and
  emit it once: at the provider's own end marker, or when the stream ends. Never join by text or
  order beyond the supplied key.
- An aggregate over a stream with a lost record (gap, malformed or over-budget chunk, partial
  event, no end marker) is still emitted, but says so: `stream_complete` and a count of
  unreadable events. Kept aggregate text stays a prefix: at the first lost record stop appending
  (to every choice and call, since a lost chunk cannot be attributed), keep counting the bytes,
  and mark it truncated. Splicing text or arguments across a hole is fabrication, and dropping
  everything read is waste.
- A streamed element that carries no key (a tool call with neither `index` nor `id`) is an
  `unknown` and a hole. Merge by position only where each entry is whole (a non-stream body).
  Identity fields (`id`, name) are taken once, never concatenated.
- Kept aggregate text counts against the decoder-wide buffer budget and is released when emitted.
  When that budget is spent the SSE framer refuses further events as `over_budget`, so a cut
  shows up as a lost event as well as a truncated field.
- An end marker (`[DONE]`) closes the record set. Whatever comes after it is a finding about the
  stream: emit one `unknown` observation (reason `data_after_done`, with `events` and `bytes`),
  and produce no decoded fact from the trailing data.

## Must and must not

Must:
- Make `accepts` deterministic over `StreamOpen` and never select by host. State exactly which
  fields it uses. It may accept an unsupported source or `content_type` to emit `unknown`, but
  must not parse that source or content as supported framing.
- Decode a logical record only when complete. The record is the unit the format frames: a JSON
  body, or one SSE event. A `Gap`, a non-`complete` `End`, a `Recovery` naming the producer, a
  stream still open at `finish`, or an over-budget record becomes one `unknown` with its
  `reason`, and no prefix of that record is parsed. Records of an SSE stream that were complete
  before the cut are still decoded. A pending partial event is an unknown; Chat Completions may
  also emit one stream-level early-end unknown after a fully dispatched event so the selected
  stream's cut cannot disappear. After a gap inside SSE, discard through the next blank line.
- Apply the single D45 interrupted-stream ruling below when a stream ends early.
- A parse failure in a record touched by an audit span is `unknown` reason `redacted` with
  the audit kinds; otherwise `malformed`.
- Bound every buffer: per record, all open streams together (including aggregate text, see
  above), `MAX_OPEN_STREAMS`, entries of every kept list, kept text cut with its full length and
  a `*_truncated` flag. Report-level bounds are in the format spec. Every id, name or code that
  came from the wire is a token or an identifier by the shared helpers, or it is withheld.
- Reference sources: `SourceRef` (stream, first and last part) and the observed interval of
  the contributing segments.
- Bump `DECODER_VERSION` on any change of interpretation.

Must not:
- Fabricate: no summed or defaulted usage, no join by text or message order, no task-success
  claim from a provider outcome.
- Keep error message text or anything credential-shaped; only safe `code`/`type` tokens.
- Panic on any input, read a clock, open a network connection, run a model or read a key.

## The redaction marker

Source: `marker()` in `crates/goalcup-daemon/src/redact.rs`; streaming policy in
`crates/goalcup-daemon/src/redact/stream.rs`.

- The marker is `<redacted:KIND>`. KIND is the rule's kind: 1 to 64 ASCII letters, digits,
  `_` or `-`.
- A capture-group span inside a JSON string interior gets the bare marker: still valid JSON.
- A capture-group span that was a whole non-string value (`17.5`, `{…}`) gets
  `"<redacted:KIND>"`, with its own quotes: still JSON, but the value is now a string.
- Whole-match spans (vendor key prefixes, bearer values, shell/TOML literals) get the bare
  marker.
- **It can break JSON.** When a stream's hold exceeds `MAX_HELD_BYTES` or its candidate limit,
  it fails closed: every byte from the earliest undecided one to the next `\n` that no
  candidate reaches, or to the stream's `End`/`Gap`, becomes one marker. JSON structure inside
  that range is gone. This is why `redacted` unknowns exist.
- `spikes/redact.py` writes a different shape, `<redacted:KIND:first8ofsha256>`. Do not
  confuse the two when reviewing a fixture.

## Fixtures

- With a real sealed trace (local, not in git): derive a fixture with a `derive.py` beside it
  that checks the source `SOURCE_SHA256`, and a `provenance.json` listing every
  transformation. Models: `crates/goalcup-verifier/tests/fixtures/m2-openai/` and `m2-anthropic/`.
- With no sanitized real trace in git (Hermes today): build the bytes in the test from the
  provider's public wire format, say so in the test header, register the decoder with
  `measured_on` naming no real traffic and every form under `synthetic_only`, and claim
  "decoder available, live row not re-verified". `m2_chat_decoder.rs` is the model. Add the
  derived fixture and move forms out of `synthetic_only` when a real trace is sanitized.
- Same-length placeholders for IDs and signatures, so offsets and segment boundaries stay
  real. Write segments as `text` for review by eye; the test loader re-encodes.
- Run `spikes/redact.py`, review by eye, then a needle scan of the committed fixture: `sk-`,
  `Bearer`, masked key forms, e-mail addresses, home paths.
- Give the connector a sample: `crates/goalcup-verifier/tests/fixtures/connectors/<id>/<name>.json`
  naming a fixture of capture records or a session file and its hint. No Rust: run
  `REGENERATE_CONNECTOR_SAMPLES=1` on `connector_samples` once, read the `expect` it writes,
  commit both. The test header (`tests/it/connector_samples.rs`) has the two forms.

## Tests

Model: `crates/goalcup-verifier/tests/it/m2_anthropic_decoder.rs` and `m2_openai_decoder.rs`, which
drive the decoder through a `DecodeSession` (`tests/common/decode.rs`), so routing is under test
too. The three wire suites share only transport builders in `tests/common/wire.rs`; provider
payload builders stay local.

- Every split: the same bytes cut at every offset, at random offsets, and one byte per segment,
  give identical facts (property-style, with a tiny deterministic generator, no dependency).
- Never a panic: random fragments of broken SSE and JSON, cut anywhere, with and without a gap
  or an `End`; `support::decode::decode_one` fails the test if the decoder panicked.
- Gap mid-record; `End` `interrupted` and `decode_error`; producer recovery; unterminated at
  `finish`.
- A redacted segment that breaks the parse; an over-budget record and the resume after it.
- Provider forms: error body, a 401 masked-key echo, tool calls by index, usage-only chunk,
  non-stream JSON.
- Control: the same bytes under another hint are ignored by your decoder (pattern:
  `the_same_bytes_under_the_responses_hint_are_selected_by_that_decoder` in
  `m2_hermes_capture_only.rs`). Repeat decode is byte-identical.
- End to end through the research report. For Hermes, T-S3-1 in `m2_hermes_capture_only.rs`
  pinned analysis `unavailable` and was rewritten with its two controls (the same bytes under
  the Responses hint, and under an unknown hint). A new decoder in a new file does the same for
  its row.
- Keep common wire record construction in `support::wire::WireSpec`; do not move
  provider-payload builders there.

## Oracles

Check decoded observations against sources the decoder did not produce: the native session
snapshot (messages, tool calls, model), the task output checked independently, supplied usage,
and the served `model` field of each response chunk.

## Records

Update the harness's facts sheet (`docs/harnesses/<id>.md`, section 9) to name the decoder,
the decoder list in the research report format (`docs/research/m2-report-format.md`), and add a
`docs/ARCHITECTURE.md` glossary entry for any new term.

## Rulings that closed old questions

- **Interrupted streams (D45, 2026-10-05; applied by all three wire decoders).** The integrity exit never
  depends on a decoder, and D45 keeps interrupted runs at exit 0 in the detail as well. A
  decoder records an `End` `interrupted` or `decode_error` as provider-specific detail and leaves
  its summary `incomplete` false when nothing was lost. A pending partial logical record is
  emitted as `unknown` and left unparsed. A provider decoder may also emit one stream-level cut
  detail after fully dispatched bytes (Chat does this, including after quiet tool deltas).
  Provider completeness fields may still say that a response or tool input was unfinished;
  that faithful provider detail is not decoder capture loss. Gaps, dropped/unsupported bytes,
  bounds, recovery and unterminated streams remain sticky decoder loss.
- **Bytes after an end marker (2026-10-05).** Everything up to `[DONE]` is complete; non-empty
  data after it emits one `unknown` observation/finding; the trailing data yields no decoded fact.
