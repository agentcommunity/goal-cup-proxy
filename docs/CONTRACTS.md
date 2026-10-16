# Capture and harness contracts

The byte and format rules the code enforces for protocol v5 capture, the local finish, the
connector contract and the control API. Types live in the source; this file states the rules
around them. Where this file and the code disagree, the code wins and this file is wrong.

Source files:

- `crates/goalcup-protocol/src/capture.rs`: records, frames, limits.
- `crates/goalcup-protocol/src/export.rs`: the chain writer, quota, terminal records.
- `crates/goalcup-protocol/src/control.rs`: the control API and the finish budget.
- `crates/goalcup-verifier/src/capture_analysis/reader.rs`: the integrity reader.
- `crates/goalcup-daemon/src/ingest.rs`, `runtime/capture/`: the receiver, the owner, the finish.
- `crates/goalcup-cli/src/proxy/`, `launch.rs`, `released_child.rs`: the recorder and the launcher.

## C1. Limits and representation

| Constant | Value | Meaning |
|---|---|---|
| `M2_PROTOCOL_VERSION` | 5 | Every envelope this build writes or reads |
| `MAX_SEGMENT_BYTES` | 64 KiB | One source chunk, and separately one sanitized segment |
| `MAX_GOAL_BYTES` | 64 KiB | The decoded goal |
| `MAX_ENVELOPE_BYTES` | 128 KiB | One serialized envelope, audit and trailing newline included |
| `MAX_INGEST_FRAME_BYTES` | 512 KiB | One encoded ingest frame; checked against its length prefix before allocation |
| `MAX_CAPTURE_QUEUE_BYTES` | 8 MiB | Sender queue memory, controls included |
| `CHAIN_QUOTA_BYTES` | 1 TiB | Logical chain quota: admission accounting, never allocated |
| `TERMINAL_RESERVE_BYTES` | 1 MiB | Part of the quota, spent only by terminal-class records |
| `MAX_OPEN_STREAMS` | 128 | Open payload streams per entry |
| `MAX_OPEN_CONNECTIONS` | 128 | Active captured connections per entry |
| `MAX_PRODUCERS` | 65,536 | Distinct producers per entry, recovered epochs included |
| `MAX_LIVE_PRODUCERS` | 128 | Producers live at once; every per-producer memory bound uses this |
| `MAX_METADATA_STRING_BYTES` | 1 KiB | One metadata string |
| metadata total | 8 KiB | All metadata strings of one stream |
| `MAX_HANDSHAKE_BYTES` | 1 KiB | The first handshake object |

Limits apply before allocation, including JSON line reads. A segment is base64 of the
**redacted** bytes. The envelope size, audit included, is checked before append.

### Records

`CaptureRecord` has ten kinds: `ProducerClosed`, `Connection`, `Recovery`, `Open`, `Segment`,
`End`, `Gap`, `Launch`, `Assistance`, `LocalFinish`. `FrameEvent` (the producer-to-daemon seam)
has seven: `Connection`, `Recovery`, `Loss`, `Open`, `Bytes`, `End`, `Gap`. Sources are
`wire_request`, `wire_response`, `ws_client`, `ws_server`, `native`, `goal`. Every capture type
denies unknown fields; a missing `Option` field reads as `None`.

`GapReason` is a closed set: `QueueFull`, `IngestLost`, `ProducerLost`, `StorageFull`,
`StorageIo`, `UnsupportedEncoding`, `DecodeFailed`, `SourceChanged`, `LimitExceeded`,
`ShutdownDeadline`.

### Identifiers and order

- A producer is a fresh random 128-bit hex id per capture process epoch. A stream is a
  producer-local counter that starts at one, in `Open` order. Ids are never reused.
- `seq` starts at zero and increments across **all** frames of one producer.
- Stream zero is reserved for, and required by, `Connection`, `Recovery` and `Loss`. A `Loss` on a
  payload stream, or any other event on stream zero, fails frame validation.
- A stream's first `Bytes` starts at offset 0. It may start later only immediately after a `Gap`
  on that stream. Otherwise offsets are contiguous.
- Wire streams carry `WireIdentity`: a random connection id kept across producer recovery, an HTTP
  exchange counter, and for WebSocket a monotonic `message_index` shared by both directions. The
  index is recorder order, not network causality.
- `offset` and `source_len` count decoded, pre-redaction source bytes. Redaction audit spans are
  absolute source offsets within that stream. `part` starts at zero per stream.
- `clock_id` is the producer id for a producer's frames. The daemon stamps its own clock on its
  own observations. Monotonic values compare only inside one `clock_id`. Cross-producer order is
  chain append order.
- Retained metadata is a whitelist: label, hint, method, host, path (no query or fragment),
  status, content type. No header, cookie, auth fingerprint or arbitrary map. A native label is
  a scoped relative path plus a generation, never an absolute path.
- Connection `Open` admits `alpn` of `None` or `http/1.1` only. A recovered socket emits `Open
  { already_open: true }` and, for WebSocket, `UpgradedWebSocket` before new payload. `Close`
  carries `Complete`, `Interrupted` or `TransportError` and no exception text.
- `Complete` means transport or file framing finished. It does not mean the agent finished.
  End-of-file on a live native file is not completion.

## C2. Ingest, redaction and storage

### Ingest wire

- Transport is loopback TCP. Each message is a big-endian `u32` length and a JSON `IngestMessage`
  (`Frame` or `Close`). The first message is a separate handshake, compact JSON
  `{"entry_id","producer","token"}` in that order, at most `MAX_HANDSHAKE_BYTES`. The golden bytes
  are `crates/goalcup-protocol/tests/fixtures/m2-ingest/golden-stream.{json,hex}`.
- The create response carries the loopback `ingest_addr` and the entry's `ingest_secret`. The
  secret never reaches argv, environment, disk or logs, and its `Debug` is redacted. A launch
  secret admits bounded fresh producer epochs for that entry only.
- Every authenticated handshake spends one of `MAX_PRODUCERS`, with or without frames.
- Frame order and stream transitions are checked before admission. `Close` follows all frames; its
  `final_seq` equals the last accepted `seq` (`None` for zero frames); no payload stream may be
  open. A valid `Close` then end-of-stream within the close grace is clean. End-of-stream without
  `Close`, a wrong `final_seq`, frames after `Close`, or an open stream at `Close` interrupts capture
  with unknown loss. `Close` is a drain boundary, not a durability acknowledgement.
- After validation the entry owner writes `ProducerClosed` with its own clock and the validated
  `final_seq`. A payload frame cannot submit it. End-of-stream without `Close` never produces it.

### Sender

- Submission is synchronous and bounded and never waits on disk or network. `Queued` means
  volatile acceptance only. There is no durable acknowledgement and no replay.
- `QueueFull` or `IngestLost` disables the old epoch and leaves a sticky `Gap`. Control capacity
  (`CONTROL_RESERVE_BYTES`, 512 KiB) sits outside the payload lane.
- Reconnect delays are 1, 2, 5, then 10 s. The sender stops after `MAX_CONNECTION_FAILURES` (10)
  failed attempts or unexpected socket losses in a row; an epoch that took items and lived
  `STABLE_EPOCH` (60 s) ends the row. `QueueFull` does not count. Each attempt's connect and
  handshake write is bounded by `CONNECT_ATTEMPT_TIMEOUT` (2 s).
- A pre-handshake retry may keep the pending producer id. After an ambiguous handshake or an
  established socket failure the id is never reused.
- At the next HTTP request or WebSocket message boundary the emitter rotates ids and emits
  `Recovery` before payload, then re-observes active connections. No interrupted body suffix is
  resumed. A refused `Recovery` leaves the emitter `Disabled`. A recovered `Recording` status does
  not clear history gaps.

### Recovery supersession (CP-A4)

- A `Recovery` by fresh producer R supersedes its `previous_producer` Q. If Q has no record in the
  chain, Q is admitted as a lost zero-record producer (an orphan recovery). The same `Recovery`
  also supersedes every other **wire** producer that is neither closed nor lost.
- Native and goal producers are never superseded. `Open { already_open: true }` never transfers a
  connection. One proxy (one wire lineage) per entry is assumed.
- A `Recovery` is admitted only as `seq` 0, naming a live or lost producer of the same launch
  that no earlier `Recovery` superseded.
- The writer refuses any record of a producer its own `Recovery` superseded, and refuses a second
  `Recovery` of one producer. The reader reports the same cases as `record_from_lost_producer`,
  `producer_recovered_twice` and `recovery_after_producer_closed`. A change to either rule must keep
  `a_loss_only_epoch_is_recoverable` (daemon) and `a_loss_only_epoch_reads_valid_and_incomplete`
  (verifier) passing.
- The receiver counts a lost producer's open streams until a `Recovery` supersedes it.

### Redaction

- `goalcup_daemon::redact::stream::StreamRedactor` is one instance per producer. It holds at most
  `MAX_HELD_BYTES` (16 KiB) of undecided suffix and flushes safe bytes within 250 ms through
  owner-driven calls. It keeps original observation times.
- Past the hold bound it fails closed: an undecided candidate or keyword context is replaced from
  its earliest byte to the next newline (or `End`, `Gap`, interrupt), with one marker.
- On interrupt an unresolved suffix is dropped and a gap is reported.
- A connector's `redaction.extra_patterns` are typed forms (a JSON field with a value form, or a
  header prefix), never a regex. `StreamRedactor::with_extra_patterns` refuses a form it has no
  recognizer for. The launcher builds it at preflight and fails closed on `Err`.
- Redaction is best effort. It is not a universal secret guarantee.

### Storage and the terminal allowance

- Chain bytes are budgeted by actual serialized size against `CHAIN_QUOTA_BYTES`. Payload stops
  `TERMINAL_RESERVE_BYTES` before the quota.
- These records spend the terminal allowance: `Gap`, `End`, `ProducerClosed`, `Assistance`,
  `LocalFinish`, connection `Close`, and the seal, cutoff and finalization terminals. Every other
  record (`Open`, `Segment`, connection `Open` and `UpgradedWebSocket`, `Recovery`, `Launch`) is
  payload. `spends_terminal_allowance` in `export.rs` is the list; any local copy matches it.
- After the first payload `CaptureStorageFull`, payload discard is sticky for the entry, and every
  later `Segment` of a stream whose payload was discarded is discarded too. `End` and stream `Gap`
  are appended only for streams whose `Open` is in the chain. After a terminal-allowance refusal
  nothing more is appended, though `LocalFinish` may still be tried. If it does not fit, the result
  is unsealed (verifier exit 1). Terminal gaps are coalesced; there is no heartbeat per dropped frame.
- Sync is bounded to 1 s and runs on finish and before upload work. An appended head is not the
  durable frontier.
- `ExportError` variants carry the refusals: `InvalidCaptureEnvelope`, `EnvelopeTooLarge`,
  `InvalidCaptureQuota`, `CaptureQuotaNotConfigured`, `CaptureStorageFull`, `CaptureRecord`,
  `LocalFinishAlreadyChained`, `TerminalManifestMismatch`.

### Crash policy

An unterminated final line makes an entry non-resumable. All bytes are preserved. The reader
reports the validated complete prefix with a torn-tail failure and a missing seal, never a full
pass. Nothing is truncated or repaired. A malformed terminated or interior record always fails.
There is no resume path: a restart reopens an unfinished entry for status and seal only (and for
abandon, if it never launched and handed nothing off).

On reopen, if the harness was registered or the launch was recorded, the daemon appends one
stream-less `Gap { IngestLost, lost_bytes: None }`: whatever the harness did while no daemon
admitted its producers is lost. A chain that already ends in its `LocalFinish` gets no gap; the
next finish commits that terminal to the registry (`SealWritten` when `seal.json` binds it,
`SealWriteFailed` when it does not).

## C3. Transport and native seams

- The proxy (`goalcup_cli::proxy`) relays HTTP/1.1 and WebSocket. It sets no base URL and changes
  upstream TLS validation not at all.
- The relay captures **decoded** bodies: chunked framing removed, at most one `gzip`, `x-gzip` or
  `deflate` coding inflated. Any other coding records an `UnsupportedEncoding` gap and no body.
  A decode failure keeps the decoded prefix and ends the stream `DecodeError`. End-of-file inside a
  body ends it `Interrupted`.
- Authentication hosts, and hosts that are not approved, are opaque tunnels and record nothing. The
  proxy peeks at SNI without consuming bytes. An approved host is intercepted, and only a path that
  matches `evidence_paths` (exact) or `prefix_paths` (prefix, query and fragment excluded) is
  captured. An empty policy captures no body. A `Host` that mismatches the SNI ends capture for
  that exchange.
- Only a valid `101` upgrade enters the WebSocket parser. A rejected upgrade stays HTTP evidence.
- `HttpOutcome` is `Closed`, `Opaque` or `WebSocket(Upgrade)`. `Opaque` means both directions ended
  and at least one stopped parsing and relayed the rest as bytes. It closes the connection
  `Complete` and counts once in `ProxyHealth.opaque_fallbacks`.
- Connection results map to `ConnectionOutcome`: `Ok` is `Complete`; `UnexpectedEof`,
  `ConnectionReset`, `ConnectionAborted`, `BrokenPipe` or a cancellation is `Interrupted`; any other
  error is `TransportError`.
- `ProxyConfig.max_connections` is within `1..=MAX_CONNECTION_TASKS` (1024). A connection past it
  is answered `503` and counted `refused_over_capacity`. Capture never refuses a connection: an
  intercepted connection takes one of the `MAX_OPEN_CONNECTIONS` table places before its `Open`. With
  none free it is relayed as bytes and counted in `capture_table_full`.
- One process-wide monotonic clock, `proxy::metadata::capture_time()`, serves all proxy
  observations.
- Native capture reads a connector's one `[[capture.native]]` source. Hints are open strings. An
  unknown hint is captured and reported as unsupported semantics.

### Proxy loss in the stream (H-16)

The proxy puts every loss in the chain at the moment it happens, so a task cut mid-relay still
leaves its mark.

- ¶1 *Wire.* `FrameEvent::Loss { reason }` is a stream-zero control. It takes the producer's next
  `seq`, enters no payload state, and marks its producer as wire, so a loss-only epoch is
  recoverable. The daemon records it as `Gap { stream: None, reason, lost_bytes: None }` on the
  producer's own clock.
- ¶2 *Emitter.* `CaptureEmitter::loss(observed, reason)` submits a `Loss` once per reason per
  emitter, from a live epoch only. A `Loss` that fails validation disables the epoch
  (`LimitExceeded`). A failed or disabled epoch submits nothing.
- ¶3 *Proxy.* `Loss { DecodeFailed }` for a fail-open tunnel is submitted at the tunnel decision,
  before the first relayed byte (`connect::admit_observed`). The same reason is used for an opaque
  fallback. `Loss { LimitExceeded }` is for a full capture table. `Loss { ShutdownDeadline }` is for
  a connection cut while unsettled; a connection cut before a tunnel or interception was decided
  submits it, and a decided tunnel does not. `ProxyHandle::stop` is the only production `try_close`
  and runs after every connection task ends.
- ¶4 *Daemon.* The owner records each admitted `Loss` as above after flushing that producer's
  decided segments. No finish stage changes.
- ¶5 *Invariant.* The first epoch's `ProducerClosed` is chained only if every loss counted in that
  epoch is a stream-less gap before it. A loss the epoch could not carry leaves a `Recovery`, a
  sticky sink gap, or an epoch that never closes. Each is coverage-incomplete to the reader. A
  later epoch exists only behind a `Recovery`, which the reader reads as incomplete.
- ¶6 *Seal request.* `CaptureSealRequest.proxy` carries the same counters as a confirmatory copy.
  The daemon appends one stream-less gap per non-zero counter before finishing: `tunneled_fail_open`
  and `opaque_fallbacks` to `DecodeFailed`, `capture_table_full` to `LimitExceeded`, `cut_unsettled`
  and `unterminated` to `ShutdownDeadline`. `sink_refusals` maps to no gap. So `gap_count` counts
  loss records and can be about double for one loss; the verdict is `gaps > 0`.
- ¶7 *Skew.* A new CLI on an older daemon: the daemon refuses `Loss` as malformed, the epoch is lost
  with a gap and then recovered; incomplete, never complete.

## C4. Status and interpretation

- `StatusPayload.capture` is `Option<M2Status>`: `run` (`Starting`, `Running`, `Exited`, `Sealed`),
  `capture` (`Starting`, `Recording`, `Incomplete`, `Failed`), `delivery`, `analysis`, `anchor`,
  `storage`, `gap_count`, `assistance_observed`. Definitions are in `capture_status.rs`. The
  projection is built before signing in every live, closed and restart path. A signed document is
  never mutated. Every document the daemon signs carries `capture`.
- The rest of the status document is small. `health` is `online`, `sealed` or
  `sealing_recovery_needed`; a registry row an older build wrote with another word reads as
  `unknown`. `arming` is `armed` (not launched yet) or `started`. There is no `event` block: the
  round's dates are the upload server's, which owns the clock.
- Gap history is sticky across finish and upload. `assistance_observed = false` means no explicit
  intervention was recorded. It is never a finding of zero assistance.
- `goalcup-verify --capture-integrity TRACE` reads protocol v5 only. Exit codes: 0 complete and
  valid within the local scope; 1 incomplete or unsealed; 2 invalid evidence, including a torn
  tail (this overrides the unsealed case); 3 usage or I/O failure with no usable report.
  Unsupported semantics alone never fails integrity. Report format:
  [m2-report-format](research/m2-report-format.md).
- A v5 line is byte-canonical: it equals the compact serialization of its decoded envelope.
- A complete v5 trace holds at least one `Launch`, each binding a goal stream (opened with source
  `goal`) that ended `Complete`, in either order. The reader keeps the goal stream each `Launch`
  names and the goal streams that ended `Complete`, each capped at `MAX_PRODUCERS`.
- `baseline.json` is bound like `seal.json`: the digest of the manifest it parses to, its file count
  and its entry equal the seq-0 `Baseline` claim.
- Reader findings (all in `reader.rs`). Incomplete (exit 1): `launch_missing`, `launch_goal_unbound`,
  `baseline_manifest_missing`, `seal_manifest_missing`. Invalid (exit 2):
  `baseline_manifest_mismatch`, `baseline_manifest_malformed`, `baseline_manifest_too_large` (over
  16 MiB), `launch_limit_exceeded`, `goal_stream_limit_exceeded`, `record_not_canonical` (an unknown
  key, whitespace or a carriage return), `recovery_names_non_wire_producer`, `seal_manifest_malformed`,
  `record_malformed`. The `Launch` check judges the validated prefix, so a trace that breaks before
  its `Launch` also lists `launch_missing`.
- Reader facts a consumer may rely on: `verified_ack`, `receipt_clock`, `last_ack_at` and `terminal`
  (`LocalFinish` or `SignedFinalization`). A missing receipt or ACK is never invented.
  `records_after_deadline` and `last_seq_at_or_before_deadline` use the participant's machine clock: a
  hint, not proof. Schema 3 adds `entry_id`, `profile_id` and `first_failure` (`{seq, code}`).
  A complete local trace may exit 0 with anchor `LocalOnly` and delivery `LocalOnly`; that asserts
  neither signed eligibility nor absence of assistance.
- Decoders are the verifier's `DecoderV1` API (`capture_analysis/api.rs`). A decoder sees the
  ordered stream of validated records, bounds its own state, emits source-referenced `unknown`
  observations for what it cannot read, and never sums overlapping observations across sources,
  cumulative usage or retries into unique totals.

## C5. Control set

`goalcup start` creates a v5 entry through these routes (types in `control.rs`):

- `POST /v0/capture-workspaces` (`CaptureWorkspaceRequest`): creates the entry. It carries the
  workspace, `profile_id`, the 64-character `connector_dir_sha256`, `reserve_bytes`, `run_home`,
  `config_home`, an optional upload-issued `entry_id`, an optional `native_since`, and
  `run_secrets` (omitted when empty): at most one value, 20 to 256 bytes of printable ASCII, the
  run's proxy credential (D60). The daemon keeps it in memory for the entry, redacts every form
  of it, and answers a bad value or a body of the wrong shape with a fixed `400 malformed` that
  quotes no value. The daemon
  rejects a config home at or below any loaded connector's `real_dirs`, and the participant's
  `HOME`, including symlink aliases, before it creates artifacts or reads a source. The answer
  carries `entry_id`, `process_generation`, `reserved_bytes`, `ingest_addr`, `ingest_secret`,
  `producer_id`, `run_public_key`, `chain_head` and `trace_dir`.
- `POST /v0/entries/{id}/capture/goal`: a one-shot preflight. It returns a stream key only after
  the redactor, append and sync succeed. The encoded request is bounded by
  `MAX_INGEST_FRAME_BYTES` and the decoded goal by `MAX_GOAL_BYTES`. An ambiguous answer aborts the
  launch; there is no retry and no duplicate append. The raw goal never appears in debug, log or
  error text. The goal producer's `ProducerClosed` is written inside the sync that precedes
  the answer.
- `POST /v0/entries/{id}/process`: registers the harness once, at `process_generation`.
- `POST /v0/entries/{id}/capture/launch`: records the `Launch` (answers `204`). The daemon fills
  `profile_id`, `profile_digest` and `recorder_revision` from the create call. No argv and no goal
  digest are recorded.
- `POST /v0/entries/{id}/seal` (`CaptureSealRequest`): the finish. It carries `proxy` counters,
  `remaining_finish_ms` and `native_scope` (below).
- `POST /v0/entries/{id}/abandon`: before the launch and before any process handoff it removes the
  entry's ingest grant, reservation, row, artifacts and empty workspace. Afterwards it is refused
  ("seal it").
- `GET /v0/entries/{id}/status`: the signed status document.

Two read routes complete the set: `GET /v0/entries` (`goalcup entries`) and `GET /v0/health`
(`goalcup doctor`, and the CLI's check that a daemon answers). There is no "current entry" route:
`goalcup status` with no id reads this machine's newest entry record.

Once an entry is finished locally, goal, launch, process registration and abandon are refused. A
finish that wrote `seal.json` records the entry's final `M2Status` on its row and releases the
entry: its ingest authorization and chain descriptors are dropped, a retried seal answers the
committed finish, and a restart does not reopen its chain. A finish whose `seal.json` was not
written keeps the entry open and `Failed`, and a restart reopens it.

At launch the daemon appends, after the `Launch` record, one stream-less `Gap { SourceChanged }` for
a required native source that is unavailable, never opened by finish, or observed to change. Native
capture failure does not stop the harness.

Native binding of a family that learns its session identity only at the end of its turn (Hermes)
is `NativeBinding::Deferred`. It becomes `Bound` at most once, installed only inside a finish at
stage 4 under the run lock. A finish with no usable scope appends one stream-less `SourceChanged`
gap at stage 5 and the binding is `Unbound`, terminal. A daemon restart forgets the binding.

## C6. Local finish

A local v5 entry needs no server. `LocalOnly` is a mode, not an unnoticed failure.
`write_local_finish` appends `LocalFinish` (it binds the digest of the canonical manifest bytes),
closes the export phase, flushes so the terminal is the durable frontier, creates `seal.json`
(absent-only) and flushes again. `LocalFinish` is once-only and v5-only; ordinary append refuses it.
An `Unresolved` termination stays explicit and never reads as a stopped run.

A failed terminal flush poisons the chain writer: `seal.json` is never written, and the reader
reports `seal_manifest_missing` (exit 1). A failed `seal.json` write leaves a durable terminal and a
missing or empty file (`seal_manifest_missing`, exit 1) or a partial one (`seal_manifest_malformed`,
exit 2). In every case a retry gets `LocalFinishAlreadyChained`; the launcher does not retry and
reports `seal_write_failed`, and the status is `Failed`, never `Sealed`.

### C6 local finish budget (H-10 4a)

- ¶1 *One budget.* `LOCAL_CAPTURE_FINISH_BUDGET` is 180 s. `FINISH_COMMIT_RESERVE` is 2 s; the
  drain share ends at `drain_end = D - FINISH_COMMIT_RESERVE`. The daemon's `FINISH_RESPONSE_MARGIN`
  (1 s) is the part of the reserve no refusable stage may begin inside. `GIT_READ_FLOOR` (30 s)
  keeps the seal of a clone's Git state from being starved by a producer that never settles or a
  connection the harness holds open.
  Termination (SIGTERM, grace, SIGKILL, kill sweep, reconfirm) is outside the budget and keeps
  [TERMINATION](TERMINATION.md)'s bound.
- ¶2 *Request.* `CaptureSealRequest.remaining_finish_ms` is the whole milliseconds left of the
  caller's budget. Absent means the full budget; larger is clamped; zero is refused as
  `finish_deadline` at stage `not_started`.
- ¶3 *CLI order.* The launcher takes `D = now + budget` when the harness has exited and calls
  `proxy.stop()` at once; nothing runs between them. The proxy's `shutdown_deadline` is
  `proxy_drain_share(budget)`: the budget less `FINISH_COMMIT_RESERVE` and `GIT_READ_FLOOR`, so
  that a connection held open at a deadline cannot spend the Git read's time (a budget too short
  for both, which only a test build names, keeps the reserve alone). A pre-seal step (the Hermes marker wait) runs after
  `proxy.stop()` and ends at `drain_end - NATIVE_FINISH_MIN` (1 s more than the reserve before
  `D`). With less time left it is skipped. The native scope it reads rides on the seal request
  (¶9). Standalone `goalcup seal` takes `D` after its status read and sends no scope.
  A run stopped while its harness still runs (its deadline, the server's stop, a full disk, a
  replacement) takes `D` at the stop instead, with the harness not exited. Its open connections (a
  streaming answer, a kept-alive WebSocket) do not end while it runs: that finish's
  `proxy.stop_within` waits 2 s (`RUNNING_HARNESS_PROXY_DRAIN`) at most, then cuts them. When the
  proxy's producer close was not queued, the sender's connection is ended too, bounded to 1 s
  (`RUNNING_HARNESS_SINK_SHUTDOWN`), so the daemon's ingest drain reads a lost producer instead of
  waiting for a live one. The seal that ends the harness is then asked within seconds of the stop.
- ¶4 *Daemon stages.* `D` starts when the daemon decodes the request. In order: (1) close
  admission and drain ingest until `drain_end`; expiry cuts with `ShutdownDeadline` gaps and is never
  a refusal. (2) blocking permit. (3) the entry's run lock; a committed finish is answered as
  committed. (4) owner stop. (5) carried proxy-counter and latched gaps are appended. (6)
  termination. (7) workspace into the seal manifest (a clone's Git state, else a walk). (8)
  `LocalFinish`, sync, `seal.json`, registry row. Stages 2, 3, 4, 6, 7 and 8 begin only while more
  than `FINISH_RESPONSE_MARGIN` remains of `D' = D + τ` (τ is time already spent in stage 6). Once
  stage 8 begins it completes under the no-retry terminal rule. A refusal is
  `DaemonError::FinishDeadline { stage }`, writes no terminal, and uses the tokens `not_started`,
  `blocking_permit`, `run_lock`, `owner_drain`, `termination`, `workspace_walk`, `local_finish`.
- ¶5 *Sticky latch.* A refusal at stage 2, 4, 6, 7 or 8 (not 3), or a request dropped before stage
  3, sets the entry's in-memory `ExpiredFinish { proxy_pending, termination }`. The next finish that
  reaches stage 5 chains the pending counters, then one stream-less `Gap { ShutdownDeadline }`,
  before `LocalFinish`. At stage 6 that finish reuses a recorded `termination` only if it is
  `Confirmed`. Otherwise it terminates again, because a harness that was parked, refused or
  unresolved may be running, and a seal never binds a run it left running. It binds the new outcome,
  except when the new outcome is `AlreadyStopped` and the recorded one is `Unresolved` with reason
  `ProcessStillRunning` or `RequestFailed`: then it binds the recorded outcome. The latch is never
  cleared by a refusal. Across a restart the reopen's `IngestLost` gap keeps the run incomplete.
  Every finish attempt sets `CaptureEntry::finishing`, which nothing clears.
- ¶6 *Transport.* `DaemonClient::seal_capture(entry_id, request, read_timeout)`. The CLI's
  `read_timeout = remaining + termination_allowance + CONNECT_TIMEOUT` (2 s).
  `termination_bound(grace) = 2·grace + 2·KILL_SWEEP_TIMEOUT_MS + RECONFIRM_DELAY_MS`, in
  `goalcup_protocol::process`. The launcher's allowance is `termination_bound(0)`. Standalone
  seal's is `termination_bound(grace)` from the cached connector, else `SLOW_CALL_TIMEOUT`. A
  transport timeout is an ambiguous outcome; a later seal answers a committed finish unchanged.
- ¶7 *Errors.* `ControlError::FinishDeadline` has code `finish_deadline`, HTTP 409, and is never
  `busy` and never retried. `CaptureSealError::NotAdmitted` is exactly `busy`, `unauthorized` and a
  connection the daemon never accepted; every other failure is `MaybeAdmitted` and is never
  replayed by the same call. `retry_capture_seal` retries only `NotAdmitted(busy)` on
  `SEAL_BUSY_BACKOFFS`, and only while the wait leaves `FINISH_COMMIT_RESERVE` of `D`. The capture
  finish never raises `busy`.
- ¶8 *Skew.* An old CLI on a new daemon gets the full budget. A new CLI on an older daemon is
  refused `malformed` before any finish step (exit 3). Nothing is finished or signalled on a
  skewed refusal.
- ¶9 *Native scope.* The scope of a family that learns it at the end of its turn rides only on a
  capture finish request (`CaptureSealRequest.native_scope`). It is installed only inside a
  finish, at stage 4, while `finishing` is set, after the committed-finish answer and before the
  owner is told to stop. A scope on any other family's finish is ignored.
- ¶10 *Evidence.* The real-binary repro rows run debug, test-feature builds of the shipped binaries
  (`pause_at` needs `test-support`); the two test-only holds are observed through seams in
  `runtime/capture/` and not in a real binary. The owner's post-stop work on a full queue was
  measured at 138 ms in a release build and 3.39 s in a debug build. The release bound is enforced
  by `owner_post_stop_drain_fits_commit_reserve` (under 500 ms). A debug daemon or a loaded host may
  refuse at `owner_drain` with no terminal and a sticky gap; a second `goalcup seal` then completes.
  A release-profile `owner_drain` refusal seen in a real run reopens the split.

### C6 capture spawn gate (H-10 4b)

Every capture child of `goalcup start` is spawned through `launch::spawn_gated`.

- ¶1 *The gate.* `spawn_gated(plan, pipes, ready_within)` forks the child and parks it before `exec`
  on a socketpair. The child writes its 4-byte pid, waits for one release byte, answers with one
  byte, and only then returns from `pre_exec`, so std `exec`s the harness. `ready_within` is
  `CAPTURE_STARTUP` (5 s) and bounds the pid read and the acknowledgement. The launcher drops its
  copy of the child's end once the pid arrives, so the parent sees EOF or `EPIPE` exactly when the
  child is dead.
- ¶2 *Registration before release.* The launcher registers the parked child through `/process`
  (the fork-time `process_start`; `exec` does not change it) and writes the release byte only after
  the daemon's `204`. A launcher that dies, a failed or ambiguous registration, or a child that dies
  while parked never leads to an `exec`. `release()` is `Ok` only on the acknowledgement and std's
  successful `exec`. Every `Err` means no harness ran, except the case in ¶5(f). So no `Launch` is
  written and the entry is sealed, never abandoned (`launch_missing`). The harness binary is not
  hashed or reopened here (D53).
- ¶3 *A seal that raced in.* A seal between registration and release cannot end the parked child:
  the daemon sees a command line that is not the harness's and answers
  `Unresolved(ProcessAuthorityUnavailable)`, signalling nothing. So the launcher reads the entry's
  signed status right after the release, and again after any `Launch` error. Only `sealed_at`
  present makes it end the child, and only its own just-released child:
  `released_child::end_released_child` sends SIGTERM (native first), waits the connector's grace,
  SIGKILLs, and returns within `termination_bound(grace)`. The launcher prints one sentence naming
  the outcome and exits 1. A status that cannot be read never ends the child. A `Conflict` on an
  entry that is still open never ends the child. The ending is not chained into the seal. The
  refusal says the harness is no longer running only for `Ended` and `AlreadyGone`. For
  `NativeUnverified` and `StillRunning` it says the harness may still be running.
- ¶4 *Native child.* A connector that names a native binary has it re-resolved on every 50 ms poll
  while the root lives, because the npm wrapper has usually not forked it when the child is
  released. It is adopted, bound by `process_start`, the first time it appears, and signalled first.
  If a native binary is named and none was ever observed, the result is `NativeUnverified`, never
  `Ended`.
- ¶5 *Residuals.* (a) A CLI crash between release and the status look, with a seal that raced in,
  leaves a running harness on a sealed entry. (a′) The same, with the CLI alive, when the status is
  unreadable twice. (b) A signal that kills the child between its acknowledgement and `exec` makes
  std report `Ok(Child)` for a dead child (a window under a microsecond). (c) Descendants other than
  the root and the re-resolved native child are never signalled by the ending. (d) The native child
  is signalled by number after an identity check, with the check-to-signal race
  `kill_retained_native` carries; a pid reused between the parentage and creation reads would be
  adopted. (e) A descendant that holds stdout open holds the launcher's tee to the finish
  deadline. (f) An acknowledgement that arrives after the 5 s read timed out but before the gate
  closes: the child `exec`s, `release()` returns `Err`, and `reap_failed_gate` SIGKILLs it with no
  grace, so a harness ran for milliseconds on a sealed entry. (g) No executable recheck (D53):
  `exec` resolves a path, not an open inode, so a replacement can change what runs; the daemon
  records the build that ran. This contract claims neither a coherent snapshot nor exact-byte
  binding.

## The v5 format freeze (harness contract S1)

- **When.** The freeze took effect at the `v0.5.0` tag.
- **What is v6.** Any new record kind, new field (optional or not), new enum value, change of
  meaning, or change to the canonical bytes of an existing record is protocol v6: a new
  `M2_PROTOCOL_VERSION`, not an amendment. Every record denies unknown fields and each v5 line must
  equal its own serialization, so a released verifier reports `record_malformed` (exit 2) on a trace
  that holds a field it does not know. That turns a valid trace invalid, which the reader rule below
  forbids. An additive v5 field needs a DECISIONS entry that accepts older verifiers failing newer
  traces.
- **The one exception (D53).** `Launch.harness_build` (`HarnessBuildV1`: `program_sha256`,
  `image_sha256`, `platform`; optional, skip-if-none) joined v5 after the tag, so the tracer records
  the build that ran instead of pinning one. A released v0.5.0 verifier refuses a trace that carries
  it. A second such field opens v6.
- **What is frozen.** `M2_PROTOCOL_VERSION = 5` is the only switch. Frozen: the `Envelope`, the
  bodies a v5 chain carries, the ten `CaptureRecord` kinds and their sub-types, the trace files
  `chain.jsonl`, `baseline.json` and `seal.json`, and `Launch` including `harness_build`. Two shapes
  inside `seal.json` are frozen knowingly: `SealManifest.workspace` is an absolute path, and
  `SealManifest.files` has no count bound. Bounding `files` later tightens the reader and needs a
  DECISIONS entry; changing either needs v6.
- **Side artifacts.** `receipt.json`, `acks.jsonl` and the `StartReceipt` and `AckCheckpoint` types
  are versioned by their own domain tags (`goalcup-receipt-v1`, ...) and are outside the record
  freeze. The v5 reader reads only the named files. An absent receipt or ack file is not an error,
  and it opens no other file in the trace directory.
- **Not frozen.** The seven `FrameEvent` kinds are the producer-to-daemon seam; a change bumps
  `CONTROL_API_VERSION`. The status projection (`capture_status.rs`) is not a record.
- **Reader rule.** A verifier change may not turn a previously valid v5 trace invalid without a
  `docs/DECISIONS.md` entry. The server is at least as new as the newest client: an unknown
  `protocol_version` is refused with `client_newer`, and the evidence stays Pending locally.
- **Guards.** Writer: the per-record goldens, the field-set table and the round-trip test in
  `goalcup-protocol/tests/it/m2_contract.rs`. Reader: the corpus
  `goalcup-verifier/tests/fixtures/v5-corpus/` with its exits, finding codes and report fields in
  `expected.toml`, run by `goalcup-verifier/tests/it/v5_corpus.rs` against the real `goalcup-verify`
  binary. A corpus expectation changes only in a PR that carries the DECISIONS entry.
- **`Launch` fields.** `profile_digest` is the SHA-256 of the `harness.toml` bytes of the profile;
  the reader does not check it. `run_public_key` is written by the daemon from the key it minted at
  create. The reader requires 64 lowercase hex that decodes to a usable Ed25519 point and, when a
  verified receipt carries a run key, that the two agree. `connector_dir_sha256` is 64 lowercase hex
  and is never compared against a pin by the reader. `allowed_tool_rule` and
  `process_handoff_digest` are written by no daemon since D52; the reader admits them by shape
  alone.

## Harness contract v1

A harness is data: `connectors/<id>/harness.toml` with `schema_version = 1`, plus one pin line.
This section names the surfaces a harness PR relies on. The rules themselves live in the files
named.

- **The schema.** [CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md) is the public field reference. Its
  executable authorities are [`harness.rs`](../crates/goalcup-protocol/src/harness.rs), the trust
  tables in [`capture_policy.rs`](../crates/goalcup-protocol/src/capture_policy.rs), the hint
  grammar in [`hints.rs`](../crates/goalcup-protocol/src/hints.rs) and folder, id and digest
  admission in [`pins.rs`](../crates/goalcup-protocol/src/pins.rs). The refusal table is
  [`tests/it/harness_schema.rs`](../crates/goalcup-protocol/tests/it/harness_schema.rs), one test per
  row.
- **Evolution of `schema_version`.** A build reads exactly `SUPPORTED_SCHEMA_VERSIONS`. An unknown
  key or closed-set value is refused with the "newer goalcup" sentence. Schema 1 stays for a new
  optional field whose absence reproduces today's behaviour, a widened bound, a new closed-set
  value or a widened refused pair. Schema 2 is for removing or renaming a field, changing a default,
  or narrowing a bound so a valid manifest fails; its PR rewrites every `harness.toml` and re-pins.
  A policy row is not schema: tightening one needs a DECISIONS entry and migrates any connector it
  trips in the same PR.
- **Pins.** `crates/goalcup-protocol/pins/connector-dirs.txt`, written only by `sh
  scripts/pin-manifests.sh` and read at build time by `pins.rs`. A pin is a tamper check after the
  build, not a defence against a malicious merge. Review is (CODEOWNERS).
- **Conformance.** `goalcup-protocol/tests/it/harness_conformance.rs` loads every connector folder
  with `load_dir` as `Origin::Pinned` and adds the checks `load_dir` does not own.
  `connector_pins.rs` compares every folder's directory digest with its pin, both ways.
- **Decoder v1.** `DecoderV1`, `DecoderRegistry` and `DecodeSession`
  ([`api.rs`](../crates/goalcup-verifier/src/capture_analysis/api.rs), `registry.rs`, `session.rs`),
  versioned by `DECODER_API_VERSION`. A harness PR never edits them. A decoder is its own PR
  ([writing-a-decoder](../skills/adding-a-capture-harness/writing-a-decoder.md)).
- **The v5 freeze** (above): the records a harness's evidence becomes.

These may change without affecting a harness: the server and its allowlist of directory digests,
upload and receipts, tracker sign-in, and the recorder's internal `FrameEvent` seam. A manifest is
delivery-agnostic: it cannot name a server, upload or tracker credential, and its evidence reaches
any consumer only as frozen v5 records plus `Launch.connector_dir_sha256`. Subscription auth stays
in the harness's isolated home and is not the tracker's.

### Migrating from manifest.toml

The daemon and CLI load `harness.toml` through the shared schema-1 loader, and nothing else. The
loader refuses a leftover `manifest.toml`. `family`, `tier`, `tested_version_range` and
`capture_only` are not harness schema fields.

No schema bump has happened. The [schema evolution rule](CONNECTOR-SCHEMA.md#pin-evolution) says
which widening stays schema 1 and which incompatible change creates schema 2. A schema-bump PR
rewrites every `connectors/*/harness.toml` and regenerates the directory pins. An older build
refuses a newer file with the "newer goalcup" sentence.

## The control API's unauthenticated surface

What a request to the control listener must carry, and which routes need no bearer. The function is
`goalcup_protocol::control::is_public_route`; the daemon and the CLI both call it, and the function
wins over this text.

- **One public route:** `GET /v0/entries/{id}/status`, the signed status document, for an id the
  caller already holds. Everything else needs `Authorization: Bearer <token from control.json>`.
  `/v0/entries` is not public: enumeration is not accepted. `/v0/assets/*`, `/v0/ui` and the removed
  `/v0/entries/current` are not public either; a request to them is an unauthenticated `401`.
- **A path with a `.` or `..` segment, or a malformed `%` escape, is never public.** The auth layer
  asks about the raw `request.uri().path()`, never a decoded one.
- **The `Host` is checked before any of this.** Exactly one `Host` line, naming the listener's own
  address: `127.0.0.1:<port>`, `localhost:<port>` or `[::1]:<port>`, where `<port>` is the port in
  `control.json` (hostname case-insensitive). A missing, repeated, foreign or differently-ported
  `Host` is refused `401` with code `unauthorized` and a fixed body that quotes nothing it was sent,
  on a public route and on a bearer route alike. This closes DNS rebinding. A client that builds its
  URL from `control.json` sends a matching `Host`.
- **Control API version.** `CONTROL_API_VERSION` is 9. `goalcup` and `goalcup-daemon` ship
  together in one release, so a pair from two releases is refused, never bridged: the CLI refuses a
  `control.json` of another version and an answer whose `x-goalcup-control-version` header differs,
  and the daemon answers a request that names another version `426 unsupported`. Version 5 added the
  required `connector_dir_sha256` to workspace creation: the daemon compares it with the connector
  it just loaded, before any allocation, and a mismatch is a path-free `422 preflight_failed`.
  Version 6 removed the retired V1 hook-path shapes, and an unknown connector is `422
  preflight_failed`. Version 7 added the optional `entry_id` to workspace creation (`e` and 16
  lowercase hex digits; any other shape is `400 malformed`), and `chain_head` and `trace_dir` to
  its answer. Version 8 removed the V1 vocabulary nothing produced: the
  `WorkspaceInfo` shape, `GET /v0/entries/current`, the status document's `event` and
  `event_status_source`, the `arming` values `start_signal_seen` and `refused`, and every `health`
  value but the three above. Version 9 added `run_secrets` to workspace creation (D60). None of
  these changes the frozen v5 chain.
