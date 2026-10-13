# M2 first-release contracts

Updated: 2026-10-11.

Paths under `.superpowers/` and `docs/superpowers/` named below are internal planning records
and are not part of the public repository. References to `LEDGER` name the retired ledger,
in git history at `6494030`.

Normative for the v0.5 implementation. These contracts are implemented and current for v0.5.
At review baseline `37e0072`, the APIs below were proposals and were not yet present; T00 and the
named owners subsequently landed the data types, traits, module declarations, contract tests and
implementation functions. Later decisions and amendments, together with current code and tests,
govern where they refine that historical task-set wording. Consumers may not rename or extend a
shared interface privately: the orchestrator merges a reviewed contract amendment before
dispatching affected consumers.

## C1. Limits and representation (T00)

In `goalcup_protocol::capture`, introduce `M2_PROTOCOL_VERSION: u32 = 5` without changing the
legacy default `PROTOCOL_VERSION = 4`. Add `EventBody::Capture { record: CaptureRecord }`.
Existing `Envelope.source` stays `String`; do not invent an incompatible source enum there.
All C1/C2 wire types derive Debug, Clone, PartialEq, Eq, Serialize, Deserialize. Use serde
snake_case enum variants, internally tagged `kind` for enums with fields. Preserve historical
serialization and reject Capture in versions 1–4. Every v5 entry has v5 baseline/seal records.

Prospective constants, not measurements:

```rust
pub const MAX_SEGMENT_BYTES: usize = 64 * 1024;
pub const MAX_GOAL_BYTES: usize = 64 * 1024; // launch resource bound, qualified by T02
pub const MAX_ENVELOPE_BYTES: usize = 128 * 1024; // includes JSON and trailing newline
pub const MAX_INGEST_FRAME_BYTES: usize = 512 * 1024; // encoded length, checked before allocation
pub const MAX_UPLOAD_BYTES: usize = 1024 * 1024; // complete serialized signed request
pub const MAX_CAPTURE_QUEUE_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_CAPTURE_RESERVE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
pub const TERMINAL_RESERVE_BYTES: u64 = 1024 * 1024; // part of, not additional to, reserve
pub const MAX_OPEN_STREAMS: usize = 128; // per entry
pub const MAX_OPEN_CONNECTIONS: usize = 128; // active captured connections per entry
pub const MAX_PRODUCERS: usize = 128; // per entry, including completed epochs
```

64 KiB bounds source chunks and sanitized segment bytes separately. Base64 happens **after
redaction** for persisted segments. Validate total envelope size including audit before append;
split emissions if necessary. Each metadata string <=1 KiB, aggregate metadata <=8 KiB. Audit
size is bounded; conservative whole-range replacement is permitted with a matching audit,
never dropping the audit silently. Limits apply before allocation, including JSON line reads.

```rust
pub struct StreamKey { pub producer: String, pub stream: u64 }
pub struct CaptureTime { pub wall: String, pub mono_ns: u64, pub clock_id: String }
pub enum CaptureSource { WireRequest, WireResponse, WsClient, WsServer, Native, Goal }
pub struct WireIdentity { pub connection: String, pub exchange: u64, pub message_index: Option<u64> }
pub enum ConnectionOutcome { Complete, Interrupted, TransportError }
pub enum ConnectionEvent {
    Open { alpn: Option<String>, already_open: bool },
    UpgradedWebSocket,
    Close { outcome: ConnectionOutcome },
}
pub struct CaptureMeta {
    pub wire: Option<WireIdentity>,
    pub label: String,
    pub protocol_hint: Option<String>,
    pub method: Option<String>,
    pub host: Option<String>,
    pub path: Option<String>,
    pub status: Option<u16>,
    pub content_type: Option<String>,
}
pub enum EndReason { Complete, Interrupted, DecodeError }
pub enum GapReason { QueueFull, IngestLost, ProducerLost, StorageFull, StorageIo,
    UnsupportedEncoding, DecodeFailed, SourceChanged, LimitExceeded, ShutdownDeadline }
pub enum FrameEvent {
    Connection { connection: String, event: ConnectionEvent },
    Recovery { previous_producer: String, reason: GapReason },
    Open { source: CaptureSource, metadata: CaptureMeta },
    Bytes { offset: u64, data: Vec<u8> },
    End { reason: EndReason },
    Gap { reason: GapReason, lost_bytes: Option<u64> },
}
pub struct CaptureFrame {
    pub stream: StreamKey,
    pub seq: u64,
    pub observed: CaptureTime,
    pub event: FrameEvent,
}
pub enum CaptureRecord {
    ProducerClosed { producer: String, observed: CaptureTime, final_seq: Option<u64> },
    Connection { producer: String, connection: String, observed: CaptureTime, event: ConnectionEvent },
    Recovery { previous_producer: String, producer: String, observed: CaptureTime, reason: GapReason },
    Open { stream: StreamKey, observed: CaptureTime,
           source: CaptureSource, metadata: CaptureMeta },
    Segment { stream: StreamKey, part: u64, source_offset: u64, source_len: u64,
              first: CaptureTime, last: CaptureTime, data_b64: String,
              audit: crate::redaction::RedactionAudit },
    End { stream: StreamKey, observed: CaptureTime, reason: EndReason },
    Gap { stream: Option<StreamKey>, observed: CaptureTime,
          reason: GapReason, lost_bytes: Option<u64> },
    Launch { observed: CaptureTime, goal_stream: StreamKey, profile_id: String,
             profile_digest: String, harness_version: String, recorder_revision: String,
             continuation: String,
             // later optional, skip-if-none additions: allowed_tool_rule,
             // process_handoff_digest, run_public_key, connector_dir_sha256 (S1)
           },
    Assistance { observed: CaptureTime, action: String },
    LocalFinish { observed: CaptureTime, reason: String,
        termination: crate::provenance::TerminationOutcomeV1, seal_manifest_sha256: String },
}
```

Producer is a fresh random 128-bit hex identifier per capture process/epoch, stream is a
producer-local monotonically increasing counter starting at one in Open emission order; IDs are never reused.
Stream zero is reserved for Connection/Recovery controls, which consume producer sequence numbers
but never enter payload stream state. Only those two FrameEvent variants may use stream zero
(superseded 2026-10-02: `Loss` is the third, see the H-16 amendment).
At most MAX_PRODUCERS distinct producers and MAX_OPEN_STREAMS simultaneously open streams per
entry are admitted; exhaustion records a gap and disables further capture, not relay. A verifier
keeps one high-water stream ID per producer plus active streams, no completed-stream database.
`seq` starts at zero and increments across **all** that producer's
frames. Wire streams carry typed `wire: Some(WireIdentity)`: connection is a fresh random
128-bit hex identifier for the physical connection, retained across producer recovery; exchange
is an HTTP request counter on that connection. Request/response share exchange. WS retains the
upgrade exchange and uses a shared monotonic message_index across both observed directions.
This is recorder observation order, not network causality. HTTP message_index is None.
Native/Goal wire is None. Labels are display text, never joins. Native label is scoped relative path + generation;
never an app-wide absolute path. `offset` and `source_len` measure decoded, pre-redaction source
bytes. Native bytes are unmodified source bytes before redaction. Redaction audit spans use
**absolute source offsets within that stream**, never base64/JSON offsets. `part` starts at
zero per stream. Timestamp range covers the input observations contributing to the segment;
clock_id is a fresh process-clock epoch (producer ID when generated by that producer); daemon
loss/control observations name the daemon clock, not a borrowed stream's clock. Monotonic values
compare only within the same clock_id. Cross-producer order is chain append order,
not a claim about causality. Complete means transport/file snapshot framing finished, not
that an agent completed its task. Tail EOF during a live native file is not completion.

Connection Open permits alpn None or `http/1.1` only. Capture HTTP/1.1 with qualified ALPN;
no HTTP/2 implementation is assumed. Recovered sockets emit Open with already_open=true and,
for WS, UpgradedWebSocket before new payload. These are current observations, not reconstructed
opening timestamps. Close records Complete/Interrupted/TransportError without exception text.
At most MAX_OPEN_CONNECTIONS active connection contexts are retained. Close releases one;
producer loss invalidates its active contexts, and Recovery re-observes surviving sockets.
Completed connection/message history is streamed, never retained in an unbounded lookup map.
Provider IDs remain decoder facts; they never substitute for recorder transport identity.

Only whitelisted metadata above is retained, through redaction. `path` excludes the entire
query and fragment. No headers, cookies, auth fingerprints, raw credential hashes or arbitrary
metadata maps. Unknown fields in a frame are rejected. Launch continuation is a sanitized
human-readable resolved setting, not dumped config. Launch goal is a Goal stream using the
same redactor. No semantic parsing is needed to retain any of these records.

## C2. Submission, privacy and storage

`goalcup_protocol::capture` (T00):

```rust
pub enum SubmitResult { Queued, Disabled(GapReason) }
pub enum EpochAdmission { Ready, Pending, RetryFresh, Disabled(GapReason) }
pub trait CaptureSink: Send + Sync {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult;
    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission;
    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult;
}
pub enum IngestMessage {
    Frame(CaptureFrame),
    Close { producer: String, final_seq: Option<u64> },
}
pub struct DurableFrontier { pub seq: u64, pub hash: String, pub chain_bytes: u64 }
pub struct CaptureStorage { pub quota_bytes: u64, pub reserved_bytes: u64, pub used_bytes: u64,
    pub durable: Option<DurableFrontier>, pub accounting_available: bool }
```

Submission is synchronous, bounded and never awaits disk/network. Queued means volatile
acceptance only. No durable ingest ACK, raw retransmission or replay. Each authenticated local
connection carries big-endian u32 length + JSON IngestMessage; its first separate handshake is
`{"entry_id":string,"producer":string,"token":string}` (<=1 KiB). A launch-scoped secret
permits bounded fresh producer epochs for that entry, not arbitrary entries. Credentials live
only in memory/private inherited descriptors, never harness env, argv, disk or logs.

Frame sequence and stream transitions are checked before admission. Close is ordered after
all frames; final_seq equals the last accepted seq (None only for zero frames), with no open
payload streams. Valid Close followed by EOF is clean. EOF without Close, wrong final_seq,
frames after Close or open streams at Close interrupt capture with unknown loss. Close is a
local drain boundary, not a durability ACK. After validation, the entry/direct-sink owner
persists ProducerClosed with its own observation clock and the validated final_seq. This is
recorder attestation to ingress validation: original frame sequences are not reconstructed from
redacted records. Payload frames cannot submit this record. The entry owner synchronizes before
final finish. EOF without Close never manufactures ProducerClosed.

QueueFull/IngestLost disable the old epoch and preserve a sticky Gap. T12 reserves bounded
control capacity outside payload admission; aggregate queue memory including controls stays
within MAX_CAPTURE_QUEUE_BYTES. It reconnects in the background with delays 1,2,5,10 seconds,
then 10 seconds, stopping after ten failed connection attempts/unexpected socket losses per proxy launch or
MAX_PRODUCERS exhaustion. Ready does not reset this failure budget; an intentional clean
shutdown is not a failed attempt. A changed producer ID cannot reset it.
No busy loops. `try_begin_epoch` returns EpochAdmission: Ready permits that ID, Pending keeps
it across polls of one attempt, RetryFresh requires a new ID after a handshake may have reached
the receiver, and Disabled means a locally known terminal failure. Pre-handshake retries can
retain the pending ID; after ambiguous handshake delivery or an established socket failure,
never reuse it. No response claims persistence. The sender sees remote EOF as unknown IngestLost,
not a distinguishable auth/storage rejection; bounded retry still applies while the receiver
refuses disabled entry/storage admission. Locally known invalid configuration/framing is terminal.

At the next new HTTP request/WS message boundary, CaptureEmitter rotates IDs and emits Recovery
before payload, re-observing active connection metadata. Old stream keys remain invalid; no
interrupted body/message suffix is resumed. Admission and sequence allocation share the emitter's
ordering lock. A failed Recovery admission leaves the new epoch disabled. Retry budgets include
failed/ambiguous attempts, and receiver epoch limits include authenticated consumed IDs.
No raw frame is replayed and a recovered Recording status cannot clear
historical Incomplete/gaps. Native capture recovery remains its explicit source-generation
policy; no synthetic native bytes are reconstructed. No automatic proxy/process restart.

Recorder-only recovery does not deliver an agent input and is not Assistance. A human resume
or repair still is Assistance. Daemon crash/torn-chain/manual entry resume follow the separate
crash policy below. Server signed chunk acknowledgements are unrelated to local IPC.

`goalcup_daemon::redact::stream` (T04):

```rust
pub struct StreamRedactor { /* private bounded state */ }
impl StreamRedactor {
    pub fn new() -> Self;
    pub fn push(&mut self, frame: CaptureFrame) -> std::io::Result<Vec<CaptureRecord>>;
    pub fn flush(&mut self, now: CaptureTime) -> std::io::Result<Vec<CaptureRecord>>;
    pub fn interrupt(&mut self, observed: CaptureTime, reason: GapReason)
        -> Vec<CaptureRecord>;
}
```

One instance per producer; the entry owner enforces the aggregate MAX_OPEN_STREAMS limit. Hold only bounded undecided suffixes;
never persist raw redactor checkpoints. Output per call is bounded by input + bounded state.
Preserve state across segments; known overlong secrets use conservative replacement until
terminator, not unbounded buffering. Flush safe bytes within 250 ms using owner-driven flush calls; retain original observation
timestamps, and never manufacture producer frames or consume their seq for ticks. Discard
unresolved suffix on interrupt and report a gap.
Production M2 supports the built-in bounded streaming policy. Arbitrary connector regexes
remain legacy-only unless separately proven streaming-safe; reject M2 profile extra_patterns
in preflight with a clear message. Best-effort body redaction is not a universal secret guarantee.

T03 adds quota/sync/local-terminal support to the existing `goalcup_protocol::export::TraceExport`:

```rust
pub fn sync_capture(&mut self) -> Result<Option<DurableFrontier>, ExportError>;
pub fn capture_storage(&self) -> CaptureStorage;
pub fn append_ack_synced(&mut self, ack: &AckCheckpoint) -> Result<(), ExportError>;
pub fn independent_acks_reader(&mut self) -> Result<TraceReadFile, ExportError>;
```

Use existing append_chain and independent_chain_reader; do not add a second journal. T03
implements `configure_capture_quota(&mut self, bytes: u64) -> Result<(), ExportError>`: explicitly unreserved
experimental accounting is never reported as physical capacity. T03R owns `reserve_capture`
with signature `reserve_capture(&mut self, bytes: u64) -> Result<(), ExportError>`
and physically reserves the actual chain file without increasing logical EOF; production
launch requires T03R. See C6 for the terminal writer contract. Budget actual
serialized bytes; payload stops before TERMINAL_RESERVE_BYTES. Coalesce terminal gaps, never
write a heartbeat per discarded frame. Sync <=1 s, on finish and before exposing upload work;
appended head is not durable frontier. All file operations use existing trace capabilities.

**Crash policy:** an unterminated final line makes an entry non-resumable in this first release.
Preserve all bytes. T18 verifies/reports the validated complete prefix with an explicit torn-tail
failure and missing seal; it never returns full integrity Pass. No silent truncation or repair.
Complete valid unsealed chains can resume only explicitly, closing previous open streams with
unknown loss and recording assistance. Malformed terminated/interior records always fail.

After the local pilot, optional T07 owns `goalcup_daemon::capture_upload`, using existing signed Chunk/AckCheckpoint types:
prepare from a capability-held chain reader only through DurableFrontier; send outside all
entry/runtime locks; commit a verified server ACK through append_ack_synced. Scan `acks.jsonl`
forward to recover delivery cursor; validate signature, entry, chain head and monotonicity.
No redb payload copies or Buffer::floor authority for v5. T07 works against a small
commit callback/store trait it defines and tests; T08 depends on both T03 and T07 to install
the real sync-backed adapter. No premature calls into unmerged methods. T07 publishes concrete prepare/send/
commit types in its merged module; T08 depends on T07 and uses those exact signatures.
This is a deliberately serial two-task seam, not an interface independent workers may guess.

## C3. Transport and native seams

New files under `goalcup_cli::proxy`, declaration owner T00. Use existing CLI binary, no new
workspace crate. `BoxIo` and configurations are defined in `proxy/api.rs` by T00:

```rust
pub trait AsyncIo: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> AsyncIo for T {}
pub type BoxIo = Box<dyn AsyncIo>;
pub struct RelayPair { pub client: BoxIo, pub upstream: BoxIo }
pub struct CapturePath { pub path: String, pub prefix: bool, pub protocol_hint: Option<String> }
pub struct CapturePolicy { pub paths: Vec<CapturePath> }
pub struct CaptureEmitter { /* private shared counters and synchronous enqueue lock */ }
impl CaptureEmitter {
    pub fn new(producer: String, sink: std::sync::Arc<dyn CaptureSink>) -> Self;
    pub fn open(&self, source: CaptureSource, metadata: CaptureMeta, observed: CaptureTime)
        -> Result<StreamKey, GapReason>;
    pub fn try_close(&self, observed: CaptureTime) -> SubmitResult;
    pub fn try_recover(&self, observed: CaptureTime) -> SubmitResult;
    pub fn connection_event(&self, connection: &str, observed: CaptureTime,
        event: ConnectionEvent) -> SubmitResult;
    pub fn emit(&self, stream: &StreamKey, observed: CaptureTime, event: FrameEvent)
        -> SubmitResult;
}
pub struct RelayContext { pub connection: String, pub host: String, pub emitter: std::sync::Arc<CaptureEmitter>,
    pub policy: CapturePolicy }
pub struct Upgrade { pub exchange: u64, pub pair: RelayPair, pub context: RelayContext,
    pub client_prefix: Vec<u8>, pub upstream_prefix: Vec<u8>,
    pub extensions: Option<String>, pub capture: Option<CaptureMeta> }
pub enum HttpOutcome { Closed, WebSocket(Upgrade) }
```

Before constructing CaptureEmitter, T13/its caller obtains Ready for the initial producer ID
using bounded asynchronous startup; the ID is shared with T12/direct-sink configuration.
CaptureEmitter::new assumes this admission and performs no I/O. Later recovery drives the
EpochAdmission state machine without waiting on the relay path.

CaptureEmitter is implemented by T00: allocate producer-wide stream/seq IDs under one short
mutex held through try_submit so concurrent relays cannot enqueue out of order. It never holds
the lock across I/O or await. FrameEvent::Open is emitted only by open; payload Bytes/End/Gap by emit; control
Connection by connection_event and Recovery only by try_recover. try_close checks no open
payload streams and calls sink.try_close_epoch under the same ordering lock; it never invents
a successful drain on timeout. Receiver/direct sink observe Close without downcasting.
RelayContext policy comes from the matched host profile; paths match exactly or by explicit
prefix, against path without query/fragment. Empty policy captures no bodies. T14 owns the
additive profile schema and publishes it before T16; no parallel consumer invents profile fields.

T09 `proxy::tls` owns CA/CONNECT admission implementation; T13 consumes its actual merged
interface. T10 exposes `proxy::http::relay_http(pair: RelayPair, context: RelayContext)
-> impl Future<Output = std::io::Result<HttpOutcome>>` as an `async fn`.
T11 exposes `proxy::websocket::relay_websocket(upgrade: Upgrade)
-> impl Future<Output = std::io::Result<()>>` as an `async fn`.
HTTP parser hands every already-read upgrade byte to WS unchanged. Upgrade.capture holds
the matched request's sanitized metadata; None means relay-only. T11 does not reconstruct path
selection from a hostname or assume every successful upgrade is evidence. Only a valid 101 accepted
upgrade enters the WS parser. Rejected upgrades stay HTTP evidence. HTTP relay preserves
wire bytes and timings, while decoded capture is a side channel. Unsupported encoding or late
decode failure emits a gap, preserves emitted prefix with uncertainty and never stores an
opaque encoded-body fallback. Authentication hosts stay opaque tunnels. Peek SNI without
consuming bytes; unknown/nonmatching SNI tunnels unchanged, Host mismatch terminates capture
for that exchange. No change to model base URL, process trust only, upstream TLS validation.

T12 owns `proxy::ingest::connect_sink` and concrete sink lifetime; T16 consumes T12.
T13 accepts Arc<dyn CaptureSink> and depends only on T09/T10/T11; E00 supplies a bounded
direct component sink, production T16 supplies the T12 adapter. No test-only fake capture path. T06
owns daemon listener construction; T15 depends on T06. Both implement C2's exact frame/handshake
fixture and CaptureSink. T05 owns `native_capture` implementation; T15 consumes its merged
interface. This keeps cross-worktree parallel seams on C1/C2, while constructors are frozen
in the owning leaf before serial integrators dispatch.

## C4. Status and interpretation

T00 adds optional `capture: Option<M2Status>` to existing StatusPayload, serde default + skip
when None; all existing literals/mocks are migrated atomically by T00. Definitions are in
`capture_status.rs`, re-exported from `capture.rs` behind a `#[doc(hidden)]` compatibility shim
tracked by post-tag ticket OS-13:

```rust
pub enum RunState { Starting, Running, Exited, Sealed }
pub enum CaptureState { Starting, Recording, Incomplete, Failed }
pub enum DeliveryState { LocalOnly, Pending, Synced, Stuck }
pub enum AnalysisState { NotRun, Available, Partial, Unavailable }
pub enum AnchorState { LocalOnly, Signed }
pub struct M2Status {
    pub run: RunState, pub capture: CaptureState, pub delivery: DeliveryState,
    pub analysis: AnalysisState, pub anchor: AnchorState, pub storage: CaptureStorage,
    pub gap_count: u64, pub assistance_observed: bool,
}
```

`Synced` describes only durable chain records through the displayed frontier, never all future
capture. Gap history is sticky across finish/upload; overflow/failure to account sets
accounting_available=false. Assistance false means no explicit intervention recorded, never a
zero-assistance finding. Decoder execution is external; analysis NotRun until a named derived
report is attached explicitly. Default daemon status does not guess whether offline analysis ran.

T00 creates verifier `capture_analysis/api.rs`; data structs derive Debug, Clone,
Serialize and Deserialize (Observation contains serde_json::Value):

```rust
pub struct SourceRef { pub stream: StreamKey, pub first_part: u64, pub last_part: u64 }
pub struct ObservationTimeRange { pub first: CaptureTime, pub last: CaptureTime }
pub struct Observation { pub source: SourceRef, pub observed: ObservationTimeRange, pub kind: String,
    pub supplied_id: Option<String>, pub fields: serde_json::Value }
pub struct DecoderSummary { pub decoder_id: String, pub decoder_version: String,
    pub observations: u64, pub unknown_records: u64, pub incomplete: bool }
pub trait Decoder {
    fn push(&mut self, record: &CaptureRecord,
            emit: &mut dyn FnMut(Observation)) -> std::io::Result<()>;
    fn finish(&mut self, emit: &mut dyn FnMut(Observation))
        -> std::io::Result<DecoderSummary>;
}
```

Observation.observed bounds the contributing retained segment observations; it is an interval,
not an invented event instant or precise model latency. Preserve separately any source-supplied
time with its source/clock. Decoders emit bounded `error` and `outcome` kinds where evidenced,
with safe supplied code/type and source references; no arbitrary headers or exception dumps.

Each decoder sees the ordered stream of all CaptureRecords; filter by source/protocol hint.
Streaming parsing is bounded; over-budget/unknown records retain source refs and unknown counts,
not fabricated zeros. `fields` contains decoded supplied facts, not guessed intent. T19 owns
native decoders; T20/T21 own provider decoders, each implementing Decoder. T22 consumes those
merged implementations. Do not sum overlapping observations across sources, cumulative usage,
or retry occurrences into unique totals. No universal lineage/authorship engine this release.
Analysis and capture integrity have independent outcomes. The legacy (pre-v5) verifier pipeline
was removed on 2026-10-12; `goalcup-verify` reads protocol v5 only.

T18 owns bounded integrity input and the explicit proposed CLI mode
`goalcup-verify --capture-integrity TRACE`; T22 adds
`goalcup-verify --research-json TRACE --output OUTPUT` using the same library offline/server.
Do not route v5 through the existing whole-trace Vec pipeline. Observation output streams to
private NDJSON with a final summary record (schema_version=1); the filename extension is
`.ndjson`, despite the command name. Bind input chain head, decoder versions and options in the
summary. Report assistance evidence and uncertainty, duration and completion separately.
No automatic public export, no credentials, no model call inside a decoder.

SourceRef applies only to actual Segment ranges. Operational records without parts contribute
to bounded summary counters/references, never fabricated segment zero. Both new CLI modes use
exit 0 complete within the explicitly local scope, 1 incomplete/partial, 2 invalid evidence (including torn tail),
3 usage/I/O failure with no usable report. Unsupported semantics alone never fails integrity.

## C5. Explicit activation and serial integration ownership

`goalcup start --goal-file PATH` selects v5 capture and requires a qualified capture profile.
Without the flag, preserve the existing v4 command path for historical compatibility; the
pilot runbook always uses the flag. Read the goal once as data; persist only its redacted Goal
stream. Manual resume selects the stored entry format and never redispatches the initial goal.
There is no `continue`/stdin relay command in this release.

T15 owns additive M2 control request/response types and routes, plus atomic CLI client/mock
methods; T16 consumes the exact merged API. Existing WorkspaceInfo stays unchanged (control API v6
later removed `WorkspaceOpts` and the V1 hook-path fields).
The M2 create path selects v5 before baseline construction. It does not depend on T14's new
profile fields; T16 supplies measured launch/profile settings when composing those two owners.
T15 owns the minimal C4 snapshot/status projection before signing in live, sealed and restart
paths, basic CLI rendering and explicit unsupported guards on legacy browser/MCP surfaces.
Optional T17 follows the pilot and extends presentation; it cannot gate early capture.

T15 adds a bounded one-shot CaptureGoalRequest { bytes: Vec<u8> } preflight control operation
returning StreamKey only after the shared redactor, append and sync succeed. T16 calls it once
before spawn; no local frame ACK or replay is introduced. Bound encoded request to
MAX_INGEST_FRAME_BYTES and decoded UTF-8 goal to MAX_GOAL_BYTES; qualify the full argv+env OS
limit in T02/T16. This is a resource limit, not the historical semantic goal-length rule.
An ambiguous control response aborts that launch; no automatic retry or duplicate goal append.
Raw goal bytes never appear in debug/log/error text or credential hashes.

Native hints are `codex-jsonl` for qualified JSONL stores with manifest family `codex`, and
`claude-code-jsonl` for qualified JSONL stores with family `claude-code`. T15 supplies these to
T05 without requiring semantic decoding; unknown family/store uses None. Provider profile
hints are `openai-responses` and `anthropic-messages`, consumed exactly by T20/T21. Strings
remain open: an unknown hint is capturable and reported as unsupported semantics.

T15 publishes `apply_capture_snapshot(payload: &mut StatusPayload, capture: Option<M2Status>)`
as a pure pre-sign projection helper in daemon status.rs. T15 calls it with actual snapshots in
every live/closed/restart path; never mutate an already signed document.

## C6. Explicit local lifecycle and verified report facts

The first pilot uses local v5 entries: no server configuration, receipt request, signed deadline,
or uploader is required. LocalOnly is a mode, not an unnoticed receipt failure. Preserve the
historical v4 path and signed clock semantics separately. Optional T08 later owns explicit signed
M2 activation and receipt-after-spawn/native-binding; never retroactively signs local duration.

T03 owns `write_cutoff_for_protocol` and `write_finalization_for_protocol` with their exact
existing arguments plus explicit protocol version, preserving old wrappers at version four.
T03 also owns the reserved terminal primitive:

```rust
pub fn write_local_finish(&mut self, manifest: &SealManifest, observed: CaptureTime,
    reason: &str, termination: crate::provenance::TerminationOutcomeV1)
    -> Result<(PathBuf, Link), ExportError>;
```

It is v5-only, validates bounded reason/termination, computes the canonical manifest digest,
binds seal.json, closes ExportPhase, and appends LocalFinish through reserved terminal authority.
Ordinary append_chain refuses LocalFinish. No signed receipt/cutoff is forged. An Unresolved
termination remains explicit and cannot yield a claimed stopped run. T15 owns create, finish,
status, explicit-stop, restart and resume behavior consuming this primitive. Terminal operation
is once-only; retry observes its committed result, never writes a duplicate terminal record.

T00 installs these shared reader facts in verifier capture_analysis/api.rs; T18 populates them
only after validation, with Debug/Clone/Serialize/Deserialize and snake_case enum variants:

```rust
pub struct VerifiedAckPosition { pub seq: u64, pub hash: String, pub acked_at: String }
pub struct VerifiedReceiptClock { pub started_at: String, pub deadline_at: String }
pub enum CaptureTerminalKind { LocalFinish, SignedFinalization, LegacySeal }
pub struct CaptureTerminalFacts {
    pub seq: u64, pub hash: String, pub observed: Option<CaptureTime>,
    pub observed_at: String, pub kind: CaptureTerminalKind, pub reason: Option<String>,
    pub termination: Option<goalcup_protocol::provenance::TerminationOutcomeV1>,
    pub seal_manifest_sha256: Option<String>,
}
```

CaptureIntegrityReport (T18) exposes verified_ack: Option<VerifiedAckPosition>,
receipt_clock: Option<VerifiedReceiptClock> and terminal: Option<CaptureTerminalFacts>.
LocalFinish retains full CaptureTime; signed/legacy wall-only evidence leaves observed None.
No absent receipt or ACK is fabricated. T22 consumes these facts, never rereads unverified files.
A valid complete local trace may exit zero with anchor LocalOnly and delivery LocalOnly;
that asserts neither signed competition eligibility nor absence of assistance.

T00 freezes historical interpretation with fixture-owned trust/pin inputs, separately from
build-relative current-manifest trust tests. Regenerating pins in T14 must not silently refresh
legacy expected findings or require historical pins to equal newly shipped profile pins.

> C6's local finish budget is amended by "Amendment 2026-10-02 (C6 local finish budget, H-10 4a)" at the end of this file.

## Orchestrator amendments, 2026-09-27

- **F1 (recovery re-observation owner).** CaptureEmitter, not each leaf, owns recovery
  re-observation: it tracks active connections from `connection_event`, and `try_recover` is
  idempotent on a live epoch (returns Queued without rotating); otherwise it rotates, emits
  Recovery, then re-emits `Open{already_open:true}`/`UpgradedWebSocket` per tracked connection
  before payload resumes. Leaves only call `try_recover` at a safe boundary. Supersedes
  CONTRACTS.md:189-191 (rotates/re-observes without an owner split), T10-http-relay.md:119-120
  and T11-websocket-relay.md:120-122 (leaf-owned re-emission).
- **F4 (result mapping and ordering).** Leaf `io::Result` maps to `ConnectionOutcome`: `Ok` ->
  Complete; `Err` of kind UnexpectedEof, ConnectionReset, ConnectionAborted, BrokenPipe, or a
  cancellation -> Interrupted; any other `Err` -> TransportError. T10/T11 return UnexpectedEof on
  mid-body/mid-frame EOF. T13 emits `UpgradedWebSocket` after `HttpOutcome::WebSocket` and before
  calling `relay_websocket`. Supersedes the unmapped returns at CONTRACTS.md:309-312.
- **F9 (torn-tail exit precedence).** A torn tail is invalid evidence and exits 2, overriding the
  unsealed=1 case. Clarifies CONTRACTS.md:397 ("2 invalid evidence (including torn tail)") against
  T18-bounded-integrity.md:74-75, which omitted the precedence.
- **CP-A1 (C2, entry owner under quota).** From the composed-checkpoint review's A1: the first
  payload `CaptureStorageFull` makes payload discard sticky for the entry, and once a stream's
  payload record has been discarded, every later Segment of that stream is discarded too; End and
  stream Gap are appended only for streams whose Open is in the chain, and Connection
  UpgradedWebSocket/Close only for connections the same producer opened or re-observed in the
  chain; after any terminal-allowance refusal, nothing more is appended, though LocalFinish may
  still be attempted, and if it does not fit the result is unsealed, exit 1; the aggregate
  MAX_OPEN_STREAMS count keeps a lost producer's streams until a persisted Recovery supersedes
  that producer, as T18 counts them. Source:
  `.superpowers/sdd/2026-09-27-m2-e01-orchestrator-task/cp1-review-1-seams.md`, "Contract
  amendments to record" A1 (from I2/M1).
- **CP-A2 (C2:193, emitter).** A refused Recovery frame makes the emitter Disabled; the
  recoverable latch applies only to refusal of a later re-observation or payload frame. Source:
  same review, A2 (from I1).
- **CP-A3 (C1).** A stream's first Bytes may start past offset 0 only immediately after a Gap on
  that stream. Source: same review, A3 (from M2).
- **F3 (T09/T13 seam split).** T09's intercept outcome is `{pair, host, policy, alpn}`; T13
  generates the connection ID and builds RelayContext. Source:
  `.superpowers/sdd/2026-09-27-m2-e01-orchestrator-task/progress.md` ledger, "Ruling F3".
- **A1-clock (capture clock).** One process-wide monotonic clock
  `proxy::metadata::capture_time()` serves all CLI observations, shared by T10 and T11; the
  emitter stamps `clock_id` as the producer ID. Source: progress.md ledger, "Ruling A1"; the seams
  review's "Clock domains" seam row (api.rs:177-190, metadata.rs:46-61).
- **T03-m1 (v5 body list).** The full v5 body list admitted for E00/E01 is
  Baseline/Capture/Seal/Cutoff/Finalization; Launch and goal records use the Capture body, and
  T15 amends the list additively. Source: progress.md ledger, "Ruling T03-m1"; the seams review's
  "T03 v5 admission" seam row (export.rs:1541-1563, 383-401).
- **GapReason (proxy health counter mapping).** The entry owner (E00/T16) maps proxy health
  counters to stream-less gaps: tunneled_fail_open -> DecodeFailed; cut_unsettled/unterminated ->
  ShutdownDeadline. Source: progress.md ledger, "Ruling (GapReason closed)".
- **T04-C1 (redaction fail-closed).** Past the 16 KiB hold bound, redaction fails closed: any
  undecided candidate/keyword context forces replacement, from the earliest undecided byte
  through the next newline (or End/Gap/interrupt), with one marker; raw bytes any rule could
  cover are never released; below-hold behaviour is unchanged. Source: progress.md ledger,
  "Ruling T04-C1".

### Wave 2 (2026-09-28)

Recorded from the wave-2 pre-flight scan
(`.superpowers/sdd/2026-09-27-m2-e01-orchestrator-task/preflight-wave2.md`, rulings R0–R19) and
the T00/T03/T04/T13 worker reports in the same directory, before T06/T12/T14/T19/T20/T21/E02
dispatch.

- **R0 (T00 amendments 1–12, merged T03/T04/T13 public APIs).** Only in ignored reports until now
  (`t00-report.md:165-234,377-401`; preflight-wave2.md:109). T00 amendments, one line each:
  1. `SubmitResult`/`EpochAdmission` derive Debug/Clone/Copy/PartialEq/Eq only — not serde types (never cross a wire).
  2. Additive derives: `Copy`+`Hash` on capture unit enums; `Hash`/`PartialOrd`/`Ord` on `StreamKey`; `PartialEq`(+`Eq`) on verifier C4/C6 structs.
  3. `deny_unknown_fields` policy on StreamKey/CaptureTime/WireIdentity/CaptureMeta/ConnectionEvent(incl. fieldless `UpgradedWebSocket` via a strict decode mirror)/FrameEvent/CaptureFrame/IngestMessage/CaptureRecord; a missing `Option` field decodes as `None`.
  4. Named constants: `MAX_METADATA_STRING_BYTES=1024`, `MAX_METADATA_BYTES=8192`, `MAX_HANDSHAKE_BYTES=1024`.
  5. Additive validation/framing API in `goalcup_protocol::capture`: `CaptureLimitError`, `is_capture_id`, `CaptureTime::{validate,mono_elapsed_ns}`, `CaptureMeta::validate`, `CaptureFrame::validate`, `IngestMessage::validate`, `checked_frame_len`, `encode_ingest_message`, `decode_ingest_message`.
  6. Handshake framing resolves the C2 ambiguity: big-endian u32 length + compact JSON `{entry_id, producer, token}` in that order, body ≤ `MAX_HANDSHAKE_BYTES`; golden `crates/goalcup-protocol/tests/fixtures/m2-ingest/golden-stream.{json,hex}`.
  7. `EnvelopeVersionCompatibilityError::CaptureRequiresV5 { protocol_version }` added.
  8. `#[allow(clippy::large_enum_variant)]` on `IngestMessage` and `HttpOutcome`, keeping the contract variants unboxed.
  9. `CaptureEmitter` local-refusal/sink-refusal/producer-budget semantics not spelled out in C2/C3, including the fix-round additions: `clock_id` is stamped to the current producer on every frame; `emit` splits oversized `Bytes` losslessly; a local refusal leaves the stream open for the caller to End/Gap.
  10. Verifier `PinTables` seam: `PinTable` alias, `PinTables{CURRENT_BUILD, manifests(), directories(), manifest_sha256(), directory_is_pinned(), fixture() under trust-fixture}`, `ManifestPinning.tables`, `ManifestPinning::resolve_with`, `Pipeline::load_with_pin_tables`.
  11. Status fixtures gain `quota-only-experiment.json` (quota > 0, reserved 0), making the quota-only rule explicit.
  12. D-5/amendment 12: the legacy `Envelope::validate_version_body_compatibility` keeps refusing protocol v5; **T03 owns v5 body admission** inside its `export.rs` lease.

  Merged public APIs consumers must build against:
  - **T03** (`crates/goalcup-protocol/src/export.rs`; `t03-report.md:101-149`): `configure_capture_quota`, `sync_capture`, `capture_storage`, `append_ack_synced`, `independent_acks_reader`, `write_cutoff_for_protocol`, `write_finalization_for_protocol`, `write_local_finish`; `ChainFormat`/`TerminalKind`/`ChainTerminal`/`ChainPhaseReplay`, `check_terminal_manifest`; new `ExportError` variants (`InvalidCaptureEnvelope`, `EnvelopeTooLarge`, `InvalidCaptureQuota`, `CaptureQuotaNotConfigured`, `CaptureStorageFull`, `CaptureRecord`, `LocalFinishAlreadyChained`, `TerminalManifestMismatch`).
  - **T04** (`crates/goalcup-daemon/src/redact/stream.rs`; `t04-report.md:33-40`): `StreamRedactor::{new, push, flush, interrupt}`, `Default` impl, `MAX_HELD_BYTES = 16 KiB`.
  - **T13** (`crates/goalcup-cli/src/proxy/mod.rs`; `t13-report.md:45-92`): `ProxyConfig`, `start()`, `StartError`, `ProxyHandle{local_addr, ca_certificate, ca_certificate_der, producer, health, stop}`, `ProducerClose`, `ProxyStopReport`, `ProxyHealth`, `ADMISSION_POLL_FIRST`/`ADMISSION_POLL_MAX`, `ACCEPT_RETRY_PAUSE`, `SHUTDOWN_CANCEL_RESERVE`.

- **R1 ACCEPTED** (amends T06:82). Recovery is admitted only as seq 0 naming a live/lost,
  not-yet-superseded producer of the same launch; T06 does not require Recovery on a later
  producer — the chain (T18) judges completeness. Every authenticated handshake consumes one of
  `MAX_PRODUCERS`, frames or not.
- **R2 ACCEPTED** (amends T06:84,88; ties to CP-A1/CP-A3 above). A stream's first Bytes at offset
  0, or — only immediately after a Gap on that stream — any offset ≥ next; otherwise exact
  contiguity. Count open streams per entry/launch, keeping a lost producer's open streams counted
  until an admitted Recovery names it.
- **R3 ACCEPTED** (amends CONTRACTS:161-162, T12:46). No shared handshake type this wave: T06 and
  T12 each define their own private 3-field struct (field order entry_id, producer, token;
  `deny_unknown_fields`; hand-written Debug omitting the token; ≤ `MAX_HANDSHAKE_BYTES`), each
  tested byte-exact against `golden-stream.hex`.
- **R4 ACCEPTED** (amends E00 sink note, T12:82). `Bytes.data` stays a JSON number array this wave
  (fixing it needs T00-owned `capture.rs`/golden/`Cargo.toml`); T06/T12 compute every bound from
  actual encoded length via `encode_ingest_message`, never a hard-coded ratio. "IPC-enc": an 8 MiB
  queue ≈ 2 MiB payload.
- **R5 ACCEPTED** (amends T06:40-41, T12:44). T12's constructor takes an explicit loopback
  `SocketAddr`, `entry_id`, in-memory secret (redacted Debug), initial producer ID and a shutdown
  deadline; T06 only documents the recommended descriptor channel. Inherited-fd creation is T15's;
  reading it is T16's.
- **R6 ACCEPTED** (amends T12:66-69). On sender-side `QueueFull`, T12 stops writing the old epoch
  at the latch, closes that socket without Close (receiver records unknown tail loss), never
  writes a partial item as complete, then follows the bounded fresh-epoch path.
- **R9 ACCEPTED** (amends T14:55-67; ties to D38). Add `home: {empty, keychain_link}` and a bounded
  `env: BTreeMap<String,String>` (denylisted names) to `CaptureProfile`; stdin-null and
  env-from-nothing stay helper invariants; the executable pin stays with detection/
  `tested_version_range`, set exactly: Codex 0.153.0, Claude 2.1.283.
- **R11 ACCEPTED** (amends T20:23, T21:23). Content-type checks apply to WireRequest/WireResponse
  only; WS streams are selected by source (WsClient/WsServer) + hint, since WS Opens carry
  `content_type: None`.
- **R12 ACCEPTED** (amends T20:55, T21:53). T19/T20/T21 get read-only access to the two raw E01
  chains (in the ignored `m2-e01-live` worktree); each extracts only its own streams, replaces IDs
  with documented placeholders, runs `spikes/redact.py` + E01's needle/pattern scan + eye review,
  and commits only under its own fixture dir with provenance (source chain sha256). Unobserved
  variants stay synthetic-only and unsupported.
- **R13 ACCEPTED** (amends T19/T20/T21 packets, silent on audit; ties to CONTRACTS:531-535 above).
  A logical record overlapping an audit span that fails to parse becomes `unknown` with bounded
  `{reason:"redacted", kinds:[…]}`, counted in `unknown_records`; never parse a truncated prefix;
  one test per decoder with a synthetic audited Segment.
- **R14 ACCEPTED** (amends index:43-44). Disclosed per-leaf private segment/SSE assemblers are
  permitted this wave (not provider parsers), nested in each leaf's own file/submodule with no
  `mod.rs` edit; consolidate into a shared `capture_analysis` helper under a lease after T22.
- **R15 ACCEPTED** (amends T03R:25). `reserve_capture(bytes)`: `fstatfs` gate
  (`f_fstypename == "apfs"`) → `configure_capture_quota`'s quota semantics → top up physical
  allocation by `bytes − (st_blocks×512 − st_size)` with `F_ALLOCATEALL|F_ALLOCATEPERSIST`,
  `F_PEOFPOSMODE`, no `ftruncate` → remeasure → only then report `reserved_bytes`; idempotent
  top-up on reopen; subsumes `configure_capture_quota` for the pilot (the quota-only path stays
  for experiments).
- **R16 ACCEPTED** (amends T03R:44-46 step 5). The disposable-volume/exhaustion/remount step is an
  `#[ignore]` test or a leased script run by hand with user permission for `hdiutil`, never in the
  default gate; small-reservation tests run on the normal APFS temp dir,
  `cfg(target_os = "macos")`, explicit-unsupported elsewhere.
- **R17 ACCEPTED** (amends T15:31 vs CONTRACTS:524-527 above). After T03R merges, T15 gets an
  explicit additive `export.rs` lease for v5 body-list growth only; T03R adds nothing speculative
  to that list.
- **R18 ACCEPTED** (amends E02:15-19,28). E02's lease gains a driver
  (`spikes/m2-first-release/early-run.sh`) and a deterministic corpus generator; the workspace is
  seeded without a runner edit via a wrapped workload
  (`/bin/sh -c 'cp -Rp "$E02_SEED"/. ./ && exec "$0" "$@"' <harness> <recipe args> -- {goal}`);
  toolchain predeclared as `/usr/bin/python3` stdlib only.
- **R19 ACCEPTED** (amends E02:59). Report timing peaks plus an externally sampled time series
  (RSS/load via the load script); full distributions are deferred as an E00-owned change.
- **R7 — MAINTAINER DECISION (differs from the scan's proposal; amends T14:80, CONTRACTS:223-224).**
  Legacy connector `[redaction].extra_patterns` ARE ported to the M2 path (the scan's proposed
  alternative — legacy-only, not applied on M2 — is superseded). New task T04b adds an additive
  `StreamRedactor` constructor taking the existing V1 extra-pattern type (reuse the daemon's
  existing type; no new schema) with the same fail-closed/bounded-hold guarantees as the built-in
  policy; T14 carries a manifest's `extra_patterns` into the capture profile unchanged and
  validates them with the same compiler T04b uses (instead of rejecting them, as T14:80 originally
  specified); T15 wires them into the per-producer redactor. Source: maintainer decision
  2026-09-28 (progress.md ledger); preflight-wave2.md R7/C7.
- **R8 — MAINTAINER DECISION (amends T14:88,93,123).** M2 capture profiles lift the V1 headless
  bans. T14 adds `capture.args` (the full measured headless launch template ending in `--`, goal
  appended as one element) validated against a capture-specific deny list (base-URL/provider-key/
  remote flags); the V1 plan path and `banned_args` stay untouched. Source: maintainer decision
  2026-09-28 (AskUserQuestion, progress.md ledger); preflight-wave2.md R8/C8.
- **R10 — MAINTAINER DECISION (amends codex `manifest.toml:149-151` `[home_seed]`).** A shared
  isolated `CODEX_HOME` (as used in E01) is used for M2 capture, not a per-entry `auth.json` copy
  — a copy risks `refresh_token_reused` invalidating the shared token on refresh (measured in
  launch-recipes §9, E01 §3.2(a)). Source: maintainer decision 2026-09-28; preflight-wave2.md
  R10/C10.

### Wave 2 implementation amendments (2026-09-28)

Recorded from the wave-2 implementation worker reports in
`.superpowers/sdd/2026-09-27-m2-e01-orchestrator-task/`, dispatched after the rulings above.

- **CP-A4 (C2, recovery supersession).** From `cp-a4-proposal.md`, "Rule" section, quoted: "A
  Recovery by fresh producer R supersedes its `previous_producer` Q... If Q has no earlier
  record..., Q is admitted as a lost zero-record producer... The same Recovery then also
  supersedes every other *wire* producer that is neither closed nor lost... Native and Goal
  producers are never wire and are never superseded by a Recovery. `Open{already_open:true}`
  never transfers a connection." Implemented in T06 (`t06-report.md`) and T18-F2
  (`t18-f2-report.md`); T15's entry owner/chain writer must mirror the same supersession set and
  interrupt each superseded producer's redactor before appending the Recovery (`cp-a4-proposal.md`
  §2, "Entry owner and chain writer" row). Assumes one proxy (one wire lineage) per entry at a
  time; two concurrent proxies in one entry would need a lineage ID first.
- **R15 text correction** (`t03r-review.md`). R15's top-up formula is corrected to "top up by
  `quota − st_blocks×512` (target `st_blocks×512 ≥ quota`)", not a second allocation on reopen;
  `reserved_bytes = min(measured, quota)`.
- **T03R** (`t03r-report.md`, `t03r-review.md`). `reserve_capture` is apfs-only: it refuses any
  filesystem whose `fstatfs` name is not exactly `apfs`. On an exhausted volume, a missing/absent
  `seal.json` reports `seal_manifest_missing` — incomplete, exit 1, an honest result, never a
  forgery. T15 must call `sync_capture()` after a failed `write_local_finish` instead of retrying,
  report `seal_write_failed`, and own its own metadata-headroom policy.
- **T04b** (`t04b-report.md` / `t04b-review.md`).
  `StreamRedactor::with_extra_patterns(extra: &[goalcup_protocol::manifest::RedactionPattern]) ->
  Result<StreamRedactor, DaemonError>`; only a regex with a reviewed `EXTRA_SHAPES` recognizer is
  accepted, so a new or edited connector regex needs its own pinned streaming recognizer; a
  stricter kind rule (`[A-Za-z0-9_-]{1,64}`) applies; open-frame metadata strings also pass through
  the connector rules. T15 must construct it at manifest load or capture preflight and fail closed
  on `Err`.
- **T06** (`t06-report.md`, `t06-review.md` declared amendments 1-5, accepted). Clean close is a
  valid `Close` then EOF within `close_grace`; a Recovery may name a sink-refused (recoverable)
  epoch; receiver-observed loss maps through `GapReason`; T15 must pass `prior_producers` on
  resume; sink methods and `IngestClock` run under the entry lock, so a T15 sink must never call
  back into `IngestEntry`. T06's ledger/stream counts cover wire (ingest) producers only; T15's
  owner sink must enforce entry-wide `MAX_PRODUCERS`/`MAX_OPEN_STREAMS` limits across all
  producers, including native and goal.
- **T12** (`t12-review.md` candidate amendments 1-5). Transport is loopback TCP, not a Unix socket
  (clarification, not a change); `with_reconnect_delays` is an additive test-only knob; delay
  timing (`CONNECT_ATTEMPT_TIMEOUT`) is measured from the end of the previous attempt/epoch and
  never reset; `QueueFull` does not count against the 10-failure budget. The CP-A2 "first-frame
  grace" is **rejected**: a refused Recovery must make the emitter `Disabled`, not `Queued` into a
  known-dead epoch.
- **T14** (`progress.md` ledger, "Ruling T14-R7" and "Ruling T14 amendments accepted"). Extra
  patterns get a shape check at profile load and are compiled with T04b's `StreamRedactor`
  constructor at T15 preflight, before launch, refusing to start on any refused pattern. The
  executable pin lives in `capture.executable` (V1's `tested_version_range` unchanged);
  `config_home=shared`, `executable` and `LaunchPlan.capture` are additive. The raw goal never
  enters `ProcessHandle`: `handle()` records `launch::GOAL_ARG_MARKER` (`"<goal>"`) at the goal's
  argv slot instead (`t14-report.md` I1).
- **Shared-home hardening (2026-10-06, D39).** `config_home=shared` preserves the exact
  `<config>/capture-homes/<connector>` leaf spelling for the create request, local entry and child
  environment. Before the request the CLI records and rechecks the leaf's `lstat` device, inode and
  type plus the canonical target spelling and the opened target's device, inode and type. The
  canonical target is opened with `O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC`, must belong to the effective
  UID, and must have exact mode `0700` (no special, group or world bits). Dangling aliases, loops,
  files, ownership/mode failures and replacements refuse before any daemon request or child. This
  does not supersede the daemon's all-loaded schema-1 `real_dirs` authority. Residual: ACLs are not
  inspected, and a same-UID process can replace an alias or target after the final recheck.
- **T19/T20/T21 decoder notes for T22** (`t19-report.md`, `t20-report.md`, `t21-report.md`). Every
  Recovery sets `incomplete`, even with no tracked stream of the lost producer, since it may hide
  an unseen Open. A request that replays history re-emits every earlier Anthropic `tool_result`
  with full content, so T22 must treat these as per-request occurrences keyed by `tool_use_id`,
  never as unique results. Synthetic-only variants (e.g. Anthropic error shapes not seen on real
  traffic) must never be presented as live-validated. The tracker's delivered goal is excluded from
  `human_message` structurally, not by text prefix: Codex uses the first `"user.text"` message on
  an `exec`-source stream; Claude Code uses the record with `promptSource == "sdk"`;
  harness-generated records after the goal (`isMeta`/`isSidechain`/`system`-sourced) are never
  classified human.

## Continuation amendments (2026-09-29)

Recorded after the code fixes that preceded T15 and T22, before those tasks dispatch. Each
blockquote names the clause it supersedes; the source is the code it names (cited by symbol) or
`docs/superpowers/plans/2026-09-29-m2-continuation-plan.md` §5. The T15, T16, T22 and T23 packets
carry their own amendments and point here for shared text.

> **Amendment 2026-09-29 (C1 bounded state, C4 integrity reader).** Supersedes C1's "no
> completed-stream database": the reader also keeps the goal stream each Launch names and the goal
> streams that ended `Complete`, each capped at `MAX_PRODUCERS`. A complete v5 trace holds at least
> one Launch, each binding a Goal stream (opened with `source: Goal`) that ended `Complete`, in
> either order. A v5 line is byte-canonical: it equals the compact serialization of its decoded
> envelope. `baseline.json` is bound like `seal.json`: the digest of the manifest it parses to, its
> file count and its entry equal the seq-0 Baseline claim. Incomplete (exit 1): `launch_missing`,
> `launch_goal_unbound`, `baseline_manifest_missing` (absent or empty). Invalid (exit 2):
> `baseline_manifest_mismatch`, `baseline_manifest_malformed`, `baseline_manifest_too_large` (over
> 16 MiB), `launch_limit_exceeded` and `goal_stream_limit_exceeded` (more than `MAX_PRODUCERS` of
> either set), `record_not_canonical` (an unknown key, whitespace or a carriage return) and
> `recovery_names_non_wire_producer` (a Recovery naming a recorded native or Goal producer, which
> CP-A4 says a Recovery never supersedes). The Launch check judges the validated prefix, so an
> invalid trace that breaks before its Launch also lists `launch_missing`.

> **Amendment 2026-09-29 (C6 local finish, T03R).** Supersedes C6's "binds seal.json, closes
> ExportPhase, and appends LocalFinish", T03R's "T15 must call `sync_capture()` after a failed
> `write_local_finish`" and its "own its own metadata-headroom policy". `write_local_finish` appends
> LocalFinish (binding the digest of the canonical manifest bytes), closes ExportPhase, flushes the
> chain so the terminal is the durable frontier, creates `seal.json` (absent-only) and flushes
> again. A failed terminal flush leaves the terminal appended but not durable and poisons the chain
> writer, so a later `sync_capture()` changes nothing; `seal.json` is never written and the reader
> reports `seal_manifest_missing`, incomplete (exit 1). A failed `seal.json` write leaves a durable
> terminal and a missing or empty `seal.json` (`seal_manifest_missing`, exit 1) or a partial one
> (`seal_manifest_malformed`, invalid, exit 2); a failed final flush leaves a complete `seal.json`
> that the reader may accept. In every case a retry gets `LocalFinishAlreadyChained`. T15 does not
> retry, does not call `sync_capture()` and has no headroom policy (`seal.json` is outside the
> chain reservation); it reports `seal_write_failed`.

> **Amendment 2026-09-29 (C2/C6 terminal allowance).** C2's "payload stops before
> TERMINAL_RESERVE_BYTES" never said which records spend the reserve. These do: `Gap`, `End`,
> `ProducerClosed`, `Assistance`, `LocalFinish`, connection `Close` and the seal, cutoff and
> finalization terminals. Every other record (`Open`, `Segment`, connection `Open` and
> `UpgradedWebSocket`, `Recovery`, `Launch`) is payload. An `Assistance` record is therefore
> admitted after `storage_full`; CP-A1's rule that nothing more is appended after a
> terminal-allowance refusal stands. Any local copy of this classification matches this list.

> **Amendment 2026-09-29 (C3 proxy outcomes and bounds).** Supersedes C3's
> `HttpOutcome { Closed, WebSocket(Upgrade) }` and the T13 bound of `max_connections` by
> `MAX_OPEN_CONNECTIONS`. `HttpOutcome` gains `Opaque`: both directions ended and at least one
> stopped parsing and relayed the rest as bytes, so nothing after that point was captured. The relay
> counts each such connection once when it falls back, however it then ends
> (`ProxyHealth.opaque_fallbacks`); `Opaque` closes the connection `Complete`, like `Closed`.
> `ProxyConfig.max_connections` bounds connection tasks within `1..=MAX_CONNECTION_TASKS`, a new
> `pub const` of 1024 in `goalcup_cli::proxy` (a relayed connection holds two descriptors); an
> accept past it is answered `503` best effort and counted `refused_over_capacity`. Capture never
> refuses a connection: an intercepted connection takes one of the `MAX_OPEN_CONNECTIONS`
> capture-table places before its `Open` (`intercepted_open` counts held places), and with none
> free it is relayed as bytes with no `Open` or `Close` and counted in `capture_table_full`.
> `ProxyHealth.sink_refusals` also counts the emitter's own refusals (an exhausted C1 bound, or a
> frame outside C1; the emitter method is crate-private, not a C3 interface).
> `ProxyStopReport::capture_loss` is true for either new counter.

> **Amendment 2026-09-29 (GapReason mapping).** Supersedes "The entry owner (E00/T16) maps proxy
> health counters to stream-less gaps" above. The mapper is the daemon's capture owner (T15), not
> E00/T16. T16 posts the proxy's raw counters in the v5 seal request; before finishing, T15 appends
> one stream-less gap (`lost_bytes: None`) per non-zero counter: `tunneled_fail_open` and
> `opaque_fallbacks` -> `DecodeFailed`, `capture_table_full` -> `LimitExceeded`, `cut_unsettled` and
> `unterminated` -> `ShutdownDeadline`. `sink_refusals` maps to no gap: the sink's refusals are
> already chained as sticky gaps, and an emitter refusal of an exhausted bound disables the epoch,
> which cannot close cleanly, so the trace shows an unclosed producer and is incomplete (the proxy
> builds its metadata within C1's bounds).

> **Amendment 2026-09-29 (R5, C2 "private inherited descriptors").** Supersedes R5's "Inherited-fd
> creation is T15's; reading it is T16's" and C2's "memory/private inherited descriptors". Nothing
> is inherited, and the daemon is not the CLI's parent. The create response carries the loopback
> `ingest_addr` and the entry's `ingest_secret` over the control API's loopback HTTP, memory to
> memory; the secret never reaches argv, environment, disk or logs, and its `Debug` is redacted.
> T12's constructor takes both explicitly, as R5 says.

> **Amendment 2026-09-29 (C5 control set).** Supersedes T15's list "create_capture_workspace,
> prepare_capture, record_capture_goal, record_capture_launch, explicit manual recovery and ordered
> producer completion" and C5's "additive M2 control request/response types and routes". The v5
> routes are: `create_capture_workspace { workspace, profile_id, reserve_bytes }` -> `{ entry_id,
> reserved_bytes, ingest_addr, ingest_secret, producer_id }` (folds `prepare_capture`; selects v5
> before the baseline and reserves before readiness); `record_capture_goal` (one-shot, as C5 says);
> `record_capture_launch { goal_stream, spawned_at, harness_version, continuation }` (the daemon
> fills `profile_id`, `profile_digest` and `recorder_revision` from the create call and appends the
> C1 Launch record; no argv and no goal digest, because the goal is an argv element and T15 retains
> no goal hash); the existing seal route with a v5 branch whose request carries the proxy counters
> and whose response is the local-finish result (the terminal, and `seal_written` or
> `seal_write_failed`); and the existing status route, which carries `M2Status` unchanged. C4 gains,
> beside `capture` and outside `M2Status`, two per-entry fields from the receiver's `EntrySnapshot`,
> `producers_consumed` and `admissions_refused`, which T16 reads before spawning. "Explicit manual
> recovery" leaves with resume. "Ordered producer completion" is the owner's internal rule, not a
> route: at most one `ProducerClosed` per producer, only from a validated close; a `Lost` outcome
> records its gap and no `ProducerClosed`.

> **Amendment 2026-09-29 (C5 status surfaces).** Supersedes C5's "explicit unsupported guards on
> legacy browser/MCP surfaces". T15 ships CLI status only. The browser and MCP guards and T15's
> `legacy_surfaces_do_not_claim_m2_health` move to T24 and land before any participant sees a page.

> **Amendment 2026-09-29 (C2 crash policy, C5 resume).** Supersedes C2's "Complete valid unsealed
> chains can resume only explicitly, closing previous open streams with unknown loss and recording
> assistance" and C5's "Manual resume selects the stored entry format". This release a v5 entry
> cannot resume: `resume` and `resume-preview` on it are refused with a plain sentence, and a
> complete valid unsealed chain can be sealed. (2026-10-12: both routes were deleted; there is no
> resume path to refuse.) C2's "a human resume or repair still is Assistance"
> stands for when resume is built. Torn chains stay preserved and non-resumable.

> **Amendment 2026-09-29 (decoder note for T22, human marker).** Supersedes "The tracker's
> delivered goal is excluded from `human_message` structurally" in the T19/T20/T21 note above.
> `human_message` needs a positive marker on the record itself; position in the stream is never
> authorship. Claude Code: `origin.kind == "human"` or `promptSource == "typed"` on a `type: "user"`
> record that is not `isMeta`, `isSidechain` or `isCompactSummary`, has no `promptSource: "system"`
> and no other `origin.kind`. Codex: a user `response_item` message whose `content_item_kinds` holds
> `user.text`, in a session whose earlier `session_meta.payload.source` is `cli` or `vscode`. Other
> user text is counted `unknown` (`unrecognized`), never human: system, meta, sidechain and compact
> records, the sdk-delivered goal (`promptSource: "sdk"`), and every Codex `exec` or unknown-source
> session. The `user.goal` label no longer exists; on an `exec` stream the first marked message,
> the delivered goal, is `unknown` with `record_type: "response_item.message.goal"`. Human
> involvement in a headless run is evidenced by `Assistance` records.

> **Amendment 2026-09-29 (decoder note for T22, Anthropic tool results).** Supersedes "re-emits
> every earlier Anthropic `tool_result` with full content" in the same note. A request
> `tool_result` keeps at most 16 KiB of its content's text. `fields.content_bytes` is the length of
> that text (a string's own, anything else's compact JSON). Within the bound `content` is as
> supplied. Over it, `content` is the text cut on a character boundary and kept as a JSON string (a
> block array becomes its cut JSON text) and `content_truncated` is `true`, so consumers read
> `content_truncated` before `content`. The Anthropic `DECODER_VERSION` is `"2"`. The per-request
> occurrence rule keyed by `tool_use_id` stands.

> **Amendment 2026-09-29 (C5 control set, T15 Checkpoint A).** Amends the "C5 control set"
> amendment above; the rest of it stands. `create_capture_workspace` takes `{ workspace,
> profile_id, reserve_bytes, run_home }` (`run_home` absolute: the per-run `HOME` the launcher
> creates, under which the daemon reads the profile's `{home}/` native stores; it is not kept on
> the row; `config_home` is added when native capture of a shared harness home is decided) and
> answers `{ entry_id, process_generation, reserved_bytes, ingest_addr, ingest_secret,
> producer_id, run_public_key }`:
> `run_public_key` is the key the caller pins to verify the public status route, and
> `process_generation` is the generation `register_process` names. `record_capture_launch`
> answers `204` with no body. The v5 seal is a fourth `DaemonClient` method,
> `seal_capture(entry_id, &CaptureSealRequest { proxy: Option<ProxyCounters> })`, with an
> `Unsupported` default, posted to the existing seal route; `proxy: None` means no proxy report
> exists. Two existing routes gain v5 meanings: `register_process` admits the entry's harness once,
> at that generation, before its launch is recorded, and without a home seed; `abandon` before the
> launch and before any process handoff is prepared removes the entry's ingest grant, reservation,
> row, artifacts and still-empty workspace, and after either is refused ("seal it"). The goal
> route appends the goal producer's `ProducerClosed { final_seq: Some(2) }` inside the sync that
> precedes its answer. Once an entry is finished locally, its goal, launch, process registration
> and abandon are refused. Each entry's v5 seal stops its own admission with `close_admission`
> and `drain(deadline)`; `IngestListener::shutdown(deadline)` runs once, at `Daemon::stop`, since
> one listener serves every entry. A finish whose `seal.json` was not written reports
> `seal_write_failed` and its status is `Failed`, never `Sealed`.

> **Amendment 2026-09-30 (C2 unread native stores, C2/C5 crash reopen, C5 sealed entries).**
> Amends "C5 control set, T15 Checkpoint A" and "C2 crash policy, C5 resume"; the rest of each
> stands.
> - A launch appends, after its `Launch` record, one stream-less `Gap { reason: SourceChanged,
>   lost_bytes: None }` for each profile `[[store]]` the entry does not read: one outside
>   `{home}/`, of a mode native capture does not take, or with a template variable it cannot
>   glob. Both shipped profiles keep their stores in a harness home outside the run `HOME`, so a
>   run of either reads incomplete (exit 1), never valid, until native capture of that home is
>   scoped (T24N, which replaces this rule with its own gap for an unavailable scoped source). A
>   profile whose stores are all under `{home}/` launches with no gap.
> - A restart reopens an unfinished entry for status and seal only (and abandon, for an entry
>   that never launched and handed nothing off; amended 2026-10-13) and admits no producer again.
>   If its harness was registered (`last_process` set) or its launch recorded (the row
>   `Started`), the reopen appends one stream-less `Gap { reason: IngestLost, lost_bytes: None }`:
>   whatever the harness did while no daemon admitted its producers is lost. `goalcup start`
>   registers the harness before the `Launch` append, so a crash between that append and the
>   row update still chains the gap. A chain that already ends in its `LocalFinish` gets no gap
>   (amended 2026-10-13): the next finish commits that terminal to the registry, as
>   `SealWritten` when `seal.json` binds it and `SealWriteFailed` when it does not.
> - A finish that writes `seal.json` records the entry's final `M2Status` on its row and releases
>   the entry: its ingest authorization and chain descriptors are dropped. Its status is answered
>   from the row, without `producers_consumed` or `admissions_refused`; a retried seal answers
>   the committed finish; goal, launch, process registration and abandon are refused; and a
>   restart does not reopen its chain. A finish whose `seal.json` was not written keeps the entry
>   open and `Failed`, and a restart reopens it. The chain's physical reservation stays allocated
>   after the finish; releasing it is a T25 item.


> **Amendment 2026-09-30 (C5 T24N native binding).** Supersedes the unread-native-store rule
> for the shipped `codex` and `claude-code` families. `CaptureWorkspaceRequest` requires
> `config_home: String` alongside `run_home`; both are absolute. The daemon rejects participant
> HOME itself and HOME/.codex or HOME/.claude descendants, including canonical/symlink aliases,
> before creating capture artifacts or reading sources. The tracker prepares isolated source
> roots before spawn. `CaptureLaunchRequest` gains serde-default `native_scope: Option<String>`.
> Codex binds the first complete `thread.started.thread_id` on stdout; Claude binds its
> preassigned session id. Codex's stdout tee scans lines up to 1 MiB, forwards every byte
> unchanged throughout the child's lifetime and drains/discards on destination closure. The
> identity wait ends by 120 seconds from spawn, stdout EOF or child exit. After child exit,
> tee completion is observed within the existing 10-second finish deadline, concurrently with
> local finish; no unbounded thread join is used. Completion confirms EOF and final forwarding
> (or draining/discarding for a closed destination). Expiry or read failure is reported to the
> participant: buffered tail delivery is then unconfirmed. A descendant-held pipe or destination
> back-pressure beyond that bound is an explicit residual, not a claim of complete delivery.
> Launch appends before
> enabling native polls; a retry neither appends another Launch nor binds a second adapter.
> Codex's one required append-only source is `config_home/sessions`, pattern
> `*/*/*/rollout-*-{id}.jsonl`, named `rollout`; Claude's is `config_home/projects`, pattern
> `*/{id}.jsonl`, named `main`. The leaf's existing 1–128 ASCII letters/digits/`-`/`_` validation
> stands. History, raw_bodies, subagents and tool-results are supplemental and excluded without
> inherited V1-store-count gaps. An absent/invalid identity, unavailable source root, required
> source never opened by finish or observed source change records one stream-less
> `Gap { reason: SourceChanged, lost_bytes: None }`; native capture failure does not stop the
> harness. Unknown non-shipping controlled-workload families retain their run-HOME store
> capture and unread-store accounting. Native and goal producers remain outside CP-A4 wire
> recovery supersession. Root/scope binding is kept in memory only; semantic judgment remains
> downstream, with no new knob, manifest/pin change or dependency.

> **Amendment 2026-10-02 (C6 local finish budget, H-10 4a).** Supersedes T16's `CAPTURE_FINISH`,
> T15's `CAPTURE_DRAIN`, the 2026-09-30 T24N phrase "the existing 10-second finish deadline", and
> C6's unbounded local finish. Ruled by D40.
>
> ¶1 *One budget.* `goalcup_protocol::control::LOCAL_CAPTURE_FINISH_BUDGET = 10 s` is the one
> local-finish budget of a v5 capture entry. `FINISH_COMMIT_RESERVE = 2 s` is its commit share;
> the drain share ends at `drain_end = D − FINISH_COMMIT_RESERVE`. The daemon's private
> `FINISH_RESPONSE_MARGIN = 1 s` is the part of that reserve no refusable stage may begin inside.
> No other finish budget exists; no limit is raised. Termination is outside the budget: SIGTERM,
> connector grace, SIGKILL, kill sweep and reconfirm keep TERMINATION.md's bound and are never cut.
>
> ¶2 *Request.* `CaptureSealRequest` gains `#[serde(default, skip_serializing_if =
> "Option::is_none")] remaining_finish_ms: Option<u64>`: whole milliseconds remaining of the
> caller's budget when the request was built. Absent = the full budget; larger = clamped; zero =
> `finish_deadline` with stage `not_started` before admission closes. Strict decoding stays.
> `CONTROL_API_VERSION`, legacy `seal`, `SealResponse` and the status route are unchanged.
>
> ¶3 *CLI order (invariant).* The launcher takes `D = now + LOCAL_CAPTURE_FINISH_BUDGET` when the
> harness has exited and calls `proxy.stop()` immediately after; nothing runs between them. The
> proxy's `shutdown_deadline` is configured to `LOCAL_CAPTURE_FINISH_BUDGET −
> FINISH_COMMIT_RESERVE`, measured from that call, so the stop ends by `drain_end` with no proxy
> API change. Any pre-seal step (Hermes S7's marker wait) runs after `proxy.stop()` returns and
> before the seal request. It is not a request of its own: the native scope it reads rides on the
> seal request (`CaptureSealRequest.native_scope`, ¶9). Its wait ends at `drain_end −
> NATIVE_FINISH_MIN` (`FINISH_COMMIT_RESERVE` plus one second before `D`; `marker_wait_end`), so
> the daemon's native read keeps a share of its drain: a wait that ran to `drain_end` would leave
> it a native budget of zero and a valid scope would still end in a gap. With less than that left
> the wait is skipped, the answer is decided from the lines already seen, and, with none, no scope
> is sent and the deferred `SourceChanged` gap stands. A slow sender is cut at the daemon's
> `drain_end` with a `ShutdownDeadline` gap, which is honest and not a refusal. The sender's
> shutdown and the scoped stdout drain get what remains of `D`. Standalone `goalcup seal` takes
> `D` after its status read and sends no scope.
>
> ¶4 *Daemon stages.* `D` starts when the daemon decodes the request. In order: (1) close
> admission and drain ingest until `drain_end`; expiry is the existing cut with `ShutdownDeadline`
> gaps, never a refusal. (2) blocking permit, (3) the entry's run lock (polled every
> `IDLE_WAIT`; a committed finish is answered as committed), (4) owner stop (native budget
> `drain_end − now`; owner exit awaited, never joined while running), (6) termination, (7)
> workspace walk, (8) `LocalFinish` + sync + `seal.json` + registry row. Stages 2, 3, 4, 6, 7 and 8
> begin only while more than `FINISH_RESPONSE_MARGIN` remains of `D' = D + τ`, where `τ` is time
> already spent in stage 6; (5) proxy-counter and latched gaps are appended between 4 and 6. Once
> stage 8 begins it completes under the existing no-retry terminal rule; only its registry row
> update is retried, by the next finish, which then appends nothing (amended 2026-10-13). A refusal is
> `DaemonError::FinishDeadline { stage }` and writes no terminal. A syscall begun inside the budget
> may overrun it; it is checked on return and no later stage starts. No hard cancellation is
> claimed. A standalone `goalcup seal` of a live harness, with the owner backlogged enough to
> reach stage 6 with less than the margin left, is refused at `termination` and needs a second
> `goalcup seal`; the second one terminates and binds the outcome.
>
> ¶5 *Sticky latch.* A refusal at stage 2, 4, 6, 7 or 8 (not 3: another finish owns the entry),
> or an attempt whose request is dropped before stage 3 (the caller's connection closed while
> the handler was draining or waiting for its permit: a guard armed once `finishing` is set and
> disarmed just before the blocking task is spawned latches it as a stage-2 refusal would),
> sets the entry's in-memory `ExpiredFinish { proxy_pending, termination }`, where
> `termination` is the outcome the refused attempt had already bound, if any. The next finish that
> reaches step 5 chains the pending proxy counters, then one stream-less `Gap { ShutdownDeadline }`,
> before `LocalFinish`. At stage 6 that finish reuses a recorded `termination` only if it is
> `Confirmed`. For any other recorded outcome it terminates again, because a harness that was
> parked, refused or unresolved may be running now, and a seal never binds a run it left running.
> It binds the new outcome, except when the new outcome is `AlreadyStopped` and the recorded one is
> `Unresolved` with reason `ProcessStillRunning` or `RequestFailed` (an attempt that signalled):
> then it binds the recorded outcome, so the trace never reads "ended on its own" over a harness
> this daemon signalled. The latch is never cleared by a refusal; a re-expiry sets it again. Across
> a restart, reopen's existing `IngestLost` keeps a registered or launched run incomplete. Every
> finish attempt also sets `CaptureEntry::finishing`, which nothing clears.
>
> ¶6 *Transport.* `DaemonClient::seal_capture(&self, entry_id, request, read_timeout: Duration)
> -> Result<CaptureFinishReport, CaptureSealError>` replaces the untimed form. The CLI's
> `read_timeout = remaining + termination_allowance + CONNECT_TIMEOUT`, where `CONNECT_TIMEOUT`
> (2 s) is slack covering request latency and `ps` spawn overhead beyond `termination_bound`
> (poll granularity excluded, as TERMINATION.md states). `termination_bound(grace) = 2·grace +
> 2·KILL_SWEEP_TIMEOUT_MS + RECONFIRM_DELAY_MS`; both constants move unchanged to
> `goalcup_protocol::process` and the daemon re-exports them. The launcher's allowance is
> `termination_bound(0)` (its root is reaped); standalone seal's is `termination_bound(grace)` from
> the cached connector manifest, falling back to `SLOW_CALL_TIMEOUT`. A transport timeout is an
> ambiguous outcome; a later seal answers a committed finish unchanged.
>
> ¶7 *Errors.* New `ControlError::FinishDeadline(String)` (appended last; code `finish_deadline`;
> HTTP 409; message = stage token) is never `busy` and never retried. `CaptureSealError::
> NotAdmitted` is exactly `busy`, `unauthorized` and a connection the daemon never accepted
> (`ureq` `ConnectionFailed`); every other failure, including handler 400s and every 404, is
> `MaybeAdmitted` and is never replayed by the same call. The capture finish never raises `busy`.
> `retry_capture_seal` retries only `NotAdmitted(busy)` on the existing `SEAL_BUSY_BACKOFFS`, and
> only while the wait leaves `FINISH_COMMIT_RESERVE` of `D`. The CLI reports `finish_deadline`,
> ambiguity and not-admitted refusals as exit-1 refusals with fixed sentences; `run_lock` says
> another finish is in progress.
>
> ¶8 *Skew.* An old CLI against a new daemon gets the full budget. A new CLI against an older
> daemon is refused `malformed` before any finish step (exit 3). An old CLI maps `finish_deadline`
> to `internal` (exit 3). Nothing is finished or signalled on any skewed refusal.
>
> ¶9 *Hermes invariants.* Every capture child is spawned only through the H-10 4b pre-exec gate,
> whatever pipes or child-only secret it carries. The native scope of a family that only learns
> it at the end of its turn (Hermes) rides only on a capture finish request
> (`CaptureSealRequest.native_scope`). There is no separate bind to refuse, and it is installed
> only inside a finish, while `CaptureEntry::finishing` is set: at stage 4, under the entry's run
> lock, after the committed-finish answer and before the owner is stopped, and only while the
> owner has not been told to stop. The binding is `NativeBinding`: `Deferred` becomes `Bound` at
> most once, so a retry after an expired finish never installs it twice; a finish that finds no
> usable scope appends exactly one stream-less `SourceChanged` gap at stage 5 and the binding is
> `Unbound`, terminal. Nothing is installed after a finish committed, a scope on any other
> family's finish is ignored, and the CLI sends none between `D` and `proxy.stop()` (¶3). A
> daemon restart forgets the binding: a reopened Hermes entry installs nothing, and its finish
> appends the deferred gap beside the reopen's own `IngestLost` unless the chain already holds it.
>
> ¶10 *Evidence scope.* The real-binary repro substitutes a whole-daemon SIGSTOP for the plan's
> held owner append and delayed blocking permit; those two holds are observed only by test-only
> seams in `runtime/capture/` (`#[cfg(any(test, feature = "test-hooks"))]`, the registry's
> precedent), not in a real binary. The real rows run the **debug, test-feature** builds of the
> shipped binaries (`pause_at` needs `test-support`); the only release-profile evidence is the
> owner measurement below. They add a real fork-and-hold-stdout descendant, and a second real
> row runs the `stream` backlog without SIGSTOP to observe a return bounded by `D`. The SIGSTOP
> row reads 1 from the verifier when the daemon dispatched the dropped request and 0 when it
> never did (the counters it carried were then never received; see ¶5 and LEDGER H-16).
>
> *Freeze measurement.* The owner's post-stop work on a full queue (8 MiB encoded, about 2 MiB of
> source: redaction, append, flush, sync) was measured as **138 ms in a release build and 3.39 s
> in a debug build**, against the roughly 0.9 s between the drain share's end and the last
> instant stage 4 may wait. The release bound is enforced (`owner_post_stop_drain_fits_commit_
> reserve` asserts under half of `FINISH_RESPONSE_MARGIN`, 500 ms, in a release build). A debug
> daemon, or a heavily loaded host, may therefore refuse at `owner_drain` after a fully spent
> drain share, with no terminal and a sticky gap; a second `goalcup seal` then completes. In the
> launcher's F1 scenario (proxy stop spent its whole 8 s) the debug owner stop took 26 to 178 ms
> in isolation and refused once under load average 15 to 21. That ending is accepted, not a
> failure; a release-profile `owner_drain` refusal seen in a dogfood or T27 run reopens the split.

> **Amendment 2026-10-02 (C6 capture spawn gate, H-10 4b).** Ruled by D40. Applies to every v5
> capture launch of `goalcup start --goal-file`. Completes ¶9 of the amendment above.
>
> ¶1 *The gate.* `launch::spawn_gated(plan, pipes, ready_within)` forks the capture child and parks
> it before `exec` on a socketpair: the child closes the parent's end, writes its 4-byte pid, waits
> for one release byte, acknowledges with one byte and only then returns from `pre_exec`, so std
> `exec`s the harness. Argv, environment, working directory and stdio are `build_command`'s, the
> same as an ungated launch. `ready_within` (the existing `CAPTURE_STARTUP`, 5 s) bounds the pid
> read and the acknowledgement. The launcher drops its copy of the child's end once the pid has
> arrived, so the parent's end sees EOF or `EPIPE` exactly when the child is dead. The Hermes path
> appends `secret: Option<&ChildSecret>`; its stderr tee is a post-release helper, never a spawner.
>
> ¶2 *Registration before release.* The launcher registers the parked child with the existing
> `/process` call (`gated_handle`: the fork-time `process_start`, which `exec` does not change, and
> no native hint) and writes the release byte only after the daemon's `204`, which follows the
> durable `Registered` row. After that response and any registered-pause test point, it re-opens
> the exact already-canonical executable path with `O_NOFOLLOW|O_NONBLOCK`, requires a regular file,
> and streams the open descriptor to EOF into SHA-256. This is not a coherent file snapshot. A
> failed open or digest mismatch aborts and reaps the parked child,
> seals the registered entry, and writes no `Launch`; no re-canonicalisation chooses a new target.
> A launcher that dies, a registration that fails or is ambiguous, a failed executable recheck, or a
> child that dies while parked never leads to an `exec`: the child reads EOF, or writes into a dead
> socket and takes SIGPIPE, and ends. `release()` is `Ok` only on the acknowledgement and std's
> successful `exec`; every `Err` means no harness ran, except the one sub-case in ¶5(f), so no
> `Launch` is written and the entry is sealed, never abandoned (`launch_missing`). A failure before the fork unmakes the entry as it
> always did. `spawn_scoped_capture` is gone; Codex's stdout tee (`tee_scoped_capture`) takes the
> pipe of a child that has already been released.
>
> ¶3 *A seal that raced in (the root ruling).* A seal between the registration and the release
> cannot have ended the child, which was still parked: the daemon's guard sees a command line that
> is not the harness's and answers `Unresolved(ProcessAuthorityUnavailable)`, signalling nothing.
> So the launcher reads the entry's signed status right after the release and again after any
> `Launch` error. Only `sealed_at` present ends the child, and only this process's own just-released
> child: `launch::end_released_child` (`released_child.rs`) sends SIGTERM native-first, waits the
> connector's grace, SIGKILLs, and returns within `termination_bound(grace)`; the launcher prints
> one sentence naming the outcome, answers the committed seal, and exits 1. A status that cannot be
> read never ends the child. A `Conflict` on an entry that is still open (an invalid allowance, a
> wrong goal stream) never ends the child. The ending is not chained into the seal. The status
> read uses the 30 s `CALL_TIMEOUT`; one that times out falls back to the look after the `Launch`
> error, which can come after Codex's native-scope wait of up to 120 s, so a seal that raced in
> can leave the harness running for that long before the launcher looks again. The final refusal
> is worded from the outcome: it says the harness is no longer running only for `Ended` and
> `AlreadyGone`, and for `NativeUnverified` and `StillRunning` it says the harness may still be
> running and points at the outcome line above it. `AlreadyGone` is reported only for a connector
> with no named native binary; the root having left alone with a named native binary is
> `NativeUnverified`. (The other ending of a released child, the tee's, is the pre-existing
> `tee_scoped_capture` failure path, a practical impossibility: it kills a child it cannot
> forward output for, whether or not the entry is sealed.)
>
> ¶4 *Native child.* The helper uses protocol functions only (the CLI cannot link the daemon).
> A connector that names a native binary has it re-resolved on **every** 50 ms poll while the root
> lives, because the npm wrapper has usually not forked it when the child is released; it is
> adopted, bound by `process_start`, the first time it appears, and signalled first. If a native
> binary is named and none was ever observed the result is `NativeUnverified`, never `Ended`.
>
> ¶5 *Residuals, stated.* (a) A CLI crash between the release and the status look, with a seal that
> raced in, leaves a running harness on a sealed entry: it is durably `Registered`, and a later
> `goalcup seal` of the sealed entry answers the committed report and signals nothing. It needs a
> manual seal racing the launch and a CLI crash within milliseconds. (a′) **The same outcome with
> the CLI alive:** a seal races in while the child is parked, and the status is then unreadable
> twice (the look after the release and the look after the `Launch` error: a dead or slow daemon,
> a read that times out after the seal committed). A status that cannot be read never ends the
> child (¶3), so the harness runs on the sealed entry. Like (a) it needs two faults. (b) A signal
> that kills the child between its acknowledgement and `exec` makes std report `Ok(Child)` for a
> dead child: a window under a microsecond that needs a signal. (c) Descendants other than the
> root and the re-resolved native child are never signalled by the ending (D30). (d) The native
> child is signalled by number after an identity check, with the check-to-signal race
> `kill_retained_native` already carries, and its parentage is read in one `ps` snapshot and its
> creation identity a moment later (a pid reused between the two would be adopted under the
> newcomer's identity; macOS pids are allocated upward). (e) A descendant that holds stdout open
> still holds the launcher's tee to the finish deadline, as before (¶3 of the 4a amendment). (f)
> **An acknowledgement that arrives after the 5 s `ready_within` read has timed out** (a parked
> child stalled for 5 s or more: SIGSTOP, or extreme load) but before the gate closes: the child
> `exec`s, `release()` returns `Err`, and `reap_failed_gate` SIGKILLs it with no grace. So "every
> `Err` means no harness ran" is exact except here, where a harness ran for milliseconds on an
> entry that is then sealed (`AlreadyStopped`). (g) The final executable recheck narrows but does
> not eliminate same-UID races on macOS. Its window begins with the first streaming read: an
> in-place writer can make the read sequence hash to the pin while the file's final bytes differ.
> After that read, `exec` resolves the path rather than the open inode, so pathname replacement can
> change what runs. This contract claims neither a coherent snapshot nor exact-byte/inode binding.

> **Amendment 2026-10-02 (C1/C3 proxy loss in the stream, H-16).** Supersedes C1's "Only those two
> FrameEvent variants may use stream zero", and makes the 2026-09-29 "GapReason mapping"'s seal-request
> counters a confirmatory copy instead of the only carrier. Amends the 4a amendment's ¶5 and ¶10
> references to LEDGER H-16. Ruled by D40 (the tracer must be safe and stable) and D3.
>
> ¶1 *Wire.* `FrameEvent` gains `Loss { reason: GapReason }` (`{"kind":"loss","reason":...}`), a
> stream-zero control. Stream zero is reserved for, and required by, `Connection`, `Recovery` and
> `Loss`. A `Loss` consumes the producer's next sequence number, enters no payload stream state, and
> marks its producer as wire in the receiver and the sink, so a loss-only epoch is recoverable.
> `IngestMessage`, `CaptureSink`, `CaptureRecord`, the envelope and `M2_PROTOCOL_VERSION` are
> unchanged. A `Loss` on a payload stream, and any other event on stream zero, fails the frame's own
> validation: the receiver loses the producer, and `CaptureEmitter::emit` refuses a `Loss` as a
> non-payload event.
>
> ¶2 *Emitter.* `CaptureEmitter::loss(observed, reason) -> SubmitResult` submits a `Loss` under the
> ordering lock, once per reason per emitter, from a live epoch only.
> - A `Loss` that fails the protocol's own validation disables the epoch (`LimitExceeded`).
> - A failed or disabled epoch submits nothing (`Disabled(reason)`); a closed epoch submits nothing
>   and counts a local refusal (unreachable in the proxy, see ¶3).
> - `count_opaque_fallback(observed)` counts and then calls `loss(..., DecodeFailed)`.
> - `open`'s over-bound metadata refusal, which leaves the epoch live, also submits
>   `Loss { LimitExceeded }` under the same lock.
> - The memory of a sent reason is the sink's volatile `Queued` answer, kept per emitter across
>   epochs. That bounds the closure records this adds to the number of reasons the proxy uses.
>
> ¶3 *Proxy.* Each loss is submitted inside the connection task that counts it, at the moment it
> begins, not when the connection ends:
> - `Loss { DecodeFailed }` for a fail-open tunnel is submitted **at the tunnel decision**, from a
>   hook `connect::admit_observed` calls with the reason before the first byte is relayed (the
>   tunnel runs inside `admit`, so a task cut or panicking mid-relay still has its loss chained).
>   The `tunneled_fail_open` counter moves in the same hook (*amended 2026-10-13*; it moved when the
>   tunnel ended, so one cut at stop is still counted). An authentication host and a
>   host that is not approved are opaque by design and submit nothing.
> - `Loss { DecodeFailed }` for an opaque fallback is submitted when the relay stops parsing.
> - `Loss { LimitExceeded }` for a full capture table is submitted before the uncaptured relay starts.
> - `Loss { ShutdownDeadline }` for a connection cut while unsettled (`Unsettled::drop`, not
>   panicking), including one cut in admission before it relayed a byte. That is conservative and
>   the same mapping the seal request's `cut_unsettled` always had; it is not to be narrowed.
>   *Amendment 2026-10-13:* the pre-decision part is unchanged: every connection cut before a
>   tunnel or interception was decided still submits this loss. Only a decided tunnel is exempt:
>   it settles in the same hook, so one still open at the cut submits nothing here. A fail-open
>   one was chained and counted (`tunneled_fail_open`) at its decision; an authentication host or
>   a host that is not approved is opaque by design.
>
> `ProxyHandle::stop` remains the only production `try_close` and runs only after every connection
> task has ended. `unterminated` and a lost accept loop submit nothing: their producer is never
> closed. A panicking `Unsettled::drop` and a panicking relay submit nothing after the decision; the
> fail-open and table-full losses were chained before the relay began.
>
> ¶4 *Daemon.* The owner records each admitted `Loss` as `Gap { stream: None, observed, reason,
> lost_bytes: None }` with the frame's own `observed` (producer clock), after flushing that
> producer's decided segments. It is a 1:1 record of a producer observation, with no judgement (D3).
> No finish stage changes. The sink marks a loss-only epoch `wire`; the `ChainWriter` cannot, because
> a stream-less gap names no producer. The two stay consistent only because the writer admits a
> `Recovery` whose predecessor is unrecorded and the reader accepts it (CP-A4): a change to either
> rule must keep `a_loss_only_epoch_is_recoverable` (daemon) and
> `a_loss_only_epoch_reads_valid_and_incomplete` (verifier) passing. *One orphan rule.* A
> predecessor is **unrecorded** (the CP-A4 orphan) when the chain holds no record under its name.
> The reader and the writer test exactly that. The receiver and the sink test its admission-side
> stand-in: the producer admitted no frame other than `Loss` (`!named`). The two are equal while the
> writer retains what the sink admitted (a writer orphan implies sink `!named`; the converse holds
> only when every admitted named frame reaches the chain); where they differ, the writer's own rule
> below keeps the chain valid. A `Recovery` naming a loss-only epoch therefore supersedes every
> other wire producer not yet closed or lost, and nothing of a superseded producer is queued or
> written afterwards. The writer also refuses any record, `ProducerClosed` included, of a producer
> its own retained `Recovery` superseded, since the reader reads one as `record_from_lost_producer`
> (invalid), and refuses a `Recovery` naming a producer already superseded or closed
> (`producer_recovered_twice`, `recovery_after_producer_closed`), so the writer alone never writes a
> record the reader refuses on these grounds.
> Tests: `a_recovery_of_a_loss_only_epoch_is_an_orphan_recovery_in_sink_and_writer`,
> `the_writer_refuses_every_record_of_a_producer_its_recovery_superseded`,
> `the_writer_refuses_a_second_recovery_of_a_producer` (daemon unit) and
> `recovery_naming_a_loss_only_epoch_supersedes_every_other_wire_producer` (receiver).
>
> ¶5 *Invariant.* The **first** proxy epoch's `ProducerClosed { final_seq }` is chained only if
> every loss counted during that epoch is a stream-less gap before it: the receiver and the sink
> admit a producer's frames in `seq` order and validate `final_seq` against the last admitted
> sequence, and the owner drains a producer's queue in FIFO order. A loss the epoch could not carry
> leaves one of three marks, each coverage-incomplete under the existing reader: a `Recovery`, the
> sink's own sticky gap (the producer is latched lost, so its close is refused), or an epoch that
> never closes. Every **later** epoch exists only behind a `Recovery`, and the reader reads
> `recoveries > 0` as incomplete. The per-emitter dedup is keyed on the sender's volatile `Queued`
> answer, which a later latch discards together with the queued `Close`; it is verdict-safe only for
> those two reasons, and it is not a claim that every later epoch carries its own copy of a loss. No
> reader rule is added, and traces chained before this amendment read unchanged.
>
> ¶6 *Seal request.* `CaptureSealRequest.proxy`, `proxy_gaps`, the 4a `AbandonGuard` and
> `ExpiredFinish.proxy_pending` stay as they are, as a confirmatory copy. A handled launcher
> request therefore adds daemon-clocked stream-less gaps for losses already chained on the
> producer's clock, so `gap_count` (the integrity report's and the signed status's) is a count of
> loss records and roughly doubles for one loss; the verdict is `gaps > 0` and is unchanged. A loss
> on a failed or disabled epoch is not wrapped as a control refusal, since it submits nothing and
> the epoch's own refusal was already counted. The CHEAT-CHECKLIST H-16 gap is closed for every
> launcher/daemon pair built from one tree: a request never dispatched, a request that loses the
> race to a committed standalone seal (the launcher is then answered from the sealed row, before
> stage zero, and exits 0 whatever its counters held), a request answered at the run lock, and a
> latch lost to a restart.
>
> ¶7 *Skew.* A new CLI on an older daemon: the older daemon refuses `Loss` as malformed and the
> epoch is lost with a gap, then recovered; incomplete, never complete. An older CLI on a new
> daemon: no `Loss`, and H-16 remains as before for that pair only.
>
> ¶8 *Evidence.* The failing-first rows are:
> - the in-process never-dispatched row (`m2_cli_lifecycle.rs`);
> - the real-binary concurrent-standalone-seal row (`capture_crash_campaign.rs`, workload mode
>   `fail-open`, test-support pause point `capture-finish-proxy-stopped`, one line after
>   `proxy.stop()` in `commands/start.rs::finish`).
>
> Both read verifier exit 0 before the change and exit 1 after. The real row runs the debug,
> test-feature builds, as 4a ¶10 states. The refusal branches the invariant rests on are pinned
> separately: the emitter (`m2_emitter.rs`), the receiver and the sink (`capture_ingest.rs`,
> `runtime/capture/tests.rs`), a loss-only epoch in the reader (`m2_capture_integrity.rs`), and a held
> fail-open tunnel whose loss is chained before the tunnel ends (`m2_proxy_integration.rs`).

## The v5 format freeze (harness contract S1)

Decided by D43 ruling 7; the design is `.superpowers/sdd/2026-10-02-oss/harness-contract-v1-design-v2.md`
§4.2. This section is the rule; the guards are named at the end.

- **When.** The freeze takes effect at the `v0.5.0` tag. Harness-contract S1 was the last
  permitted change to the v5 record shapes: it added `Launch.run_public_key` and
  `Launch.connector_dir_sha256`, both optional and skip-if-none. That was allowed only because no
  verifier had been released, so none could meet a trace it did not know.
- **What is v6.** From `v0.5.0`, any new record kind, any new field (optional or not), any new
  enum value, any change of meaning and any change to the canonical bytes of an existing record
  is protocol v6: a new `M2_PROTOCOL_VERSION`, not an amendment. **Why a new optional field is
  not an exception:** every record carries `deny_unknown_fields`, and each v5 line must equal its
  own serialization, so a released verifier reports `record_malformed` (exit 2) on a trace that
  holds a field it does not know. That turns a valid trace invalid, which the reader rule below
  forbids. Wanting additive v5 fields after the tag needs its own DECISIONS row that accepts
  older verifiers failing newer traces.
- **What is frozen.** `M2_PROTOCOL_VERSION = 5` (`capture.rs`) is the only switch. Frozen are the
  `Envelope`, the bodies a v5 chain carries, the ten `CaptureRecord` kinds and their sub-types,
  the trace layout files `chain.jsonl`, `baseline.json` and `seal.json`, and `Launch` as S1 left
  it. Two shapes inside `seal.json` are frozen knowingly (owner decision 7): `SealManifest.workspace`
  is an absolute path, often personal data, and `SealManifest.files` has no count bound. Bounding
  `files` later tightens the reader, so it needs a DECISIONS row, and changing either needs v6.
- **Side artifacts.** `receipt.json`, `acks.jsonl` and the `StartReceipt`, `AckCheckpoint` and
  `Chunk` types, and any later side file (the server phase's registration and fixed documents),
  are versioned by their own domain tags (`goalcup-receipt-v1`, `goalcup-chunk-v1`, ...) and are
  outside the record freeze. (`Chunk` and `MAX_UPLOAD_BYTES`, in `chunk.rs`, were deleted with the
  devserver on 2026-10-12: no shipped binary uploaded a chunk.) The v5
  reader reads only the named files; an absent receipt or ack file is not an error, and it opens
  no other file in the trace directory.
- **Not frozen.** The seven `FrameEvent` kinds are the producer-to-daemon seam inside one
  recorder, never chained and never seen by a harness or a server. A change to them bumps
  `CONTROL_API_VERSION` (`control.rs`), which governs CLI/daemon skew. The status projection
  (`capture_status.rs`) is likewise not a record. The ingest types (`CaptureFrame`,
  `IngestMessage`, `SubmitResult`, `EpochAdmission`) still sit in `capture.rs`; the freeze is
  by type, as this list says, not by module. Post-tag ticket OS-13 owns moving them and removing
  the compatibility shim.
- **Reader rule.** A verifier change may not turn a previously valid v5 trace invalid without a
  `docs/DECISIONS.md` row. The server is at least as new as the newest client: an unknown
  `protocol_version` is refused with `client_newer`, and the evidence stays Pending locally.
- **The guards.** Writer: the per-record goldens, the field-set table and the pre-S1 round-trip
  test in `goalcup-protocol/tests/it/m2_contract.rs`. Reader: the committed corpus
  `goalcup-verifier/tests/fixtures/v5-corpus/` with its exits, finding codes and report fields in
  `expected.toml`, run by `goalcup-verifier/tests/it/v5_corpus.rs` against the real
  `goalcup-verify` binary; CI runs it (the verifier tests are not excluded). A corpus expectation
  changes only in a pull request that carries the DECISIONS row.
- **`Launch` fields.** `profile_digest` is the SHA-256 of the `harness.toml` bytes that declared
  the capture profile; the reader does not check it.
  `run_public_key` is written by the daemon from the key it minted at create (never supplied by
  the CLI); the reader requires 64 lowercase hex that decodes to a usable Ed25519 point, and,
  when a verified receipt carries a run key, that the two agree (compared canonically). No v5
  producer writes a receipt key yet, and a receipt without one is skipped, so that check guards
  the server phase's registration, not today's traces. `connector_dir_sha256` is 64 lowercase hex,
  never compared against a pin by the reader. The daemon forwards the authoritative digest stored
  on the entry after control API v5 compared it with the launcher's once-loaded digest (D46).

## Harness contract v1

Decided by D42 and D43. A harness is data: `connectors/<id>/harness.toml`, `schema_version = 1`,
plus one pin line. This section names the surfaces a harness PR relies on and where each lives;
the rules themselves are in the files named, not restated here.

- **The schema.** The public field, default, bound, origin, loader and pin reference is
  [CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md). Its executable authorities are
  [`harness.rs`](../crates/goalcup-protocol/src/harness.rs), the trust tables in
  [`capture_policy.rs`](../crates/goalcup-protocol/src/capture_policy.rs), the open hint grammar
  in [`hints.rs`](../crates/goalcup-protocol/src/hints.rs) and folder/id/digest admission in
  [`pins.rs`](../crates/goalcup-protocol/src/pins.rs). The refusal table is
  [`tests/it/harness_schema.rs`](../crates/goalcup-protocol/tests/it/harness_schema.rs), one test per row.
- **Evolution of `schema_version`.** A build reads exactly `SUPPORTED_SCHEMA_VERSIONS`. An unknown
  key or closed-set value is refused with the "newer goalcup" sentence. Schema 1 stays for a new
  optional field whose absence reproduces today's behaviour, a widened bound, a new closed-set
  value or a widened refused pair. Schema 2 is for removing or renaming a field, changing a
  default, or narrowing a bound so a valid manifest fails; its PR rewrites every `harness.toml`
  and re-pins. A policy row is not schema: tightening one needs a DECISIONS row and migrates any
  connector it trips in the same PR.
- **Pins.** `crates/goalcup-protocol/pins/connector-dirs.txt`, data written only by
  `sh scripts/pin-manifests.sh`, read at build time by `pins.rs`. A pin is a tamper check after
  the build, not a defence against a malicious merge: review is (CODEOWNERS).
- **Conformance.** `goalcup-protocol/tests/it/harness_conformance.rs` loads every connector folder
  with `load_dir` as `Origin::Pinned` and adds the checks `load_dir` does not own;
  `connector_pins.rs` compares every folder's directory digest with its pin, both ways. CI runs
  both on a fork PR (`ci.yml` excludes only the daemon and CLI suites).
- **Decoder v1.** `DecoderV1`, `DecoderRegistry` and `DecodeSession`
  ([`api.rs`](../crates/goalcup-verifier/src/capture_analysis/api.rs),
  `registry.rs`, `session.rs`), versioned by `DECODER_API_VERSION`. A harness PR never edits them;
  a decoder is its own core-adjacent PR ([writing-a-decoder](../skills/adding-a-capture-harness/writing-a-decoder.md)).
- **The v5 freeze** ([above](#the-v5-format-freeze-harness-contract-s1)): the records a harness's evidence becomes.

**May change without affecting a harness:** the server and its allowlist of directory digests
(D43 ruling 5), upload and chunk streaming and receipts, tracker sign-in and other auth, and the
recorder's internal `FrameEvent` seam (`CONTROL_API_VERSION`). A harness manifest is
delivery-agnostic: it cannot name a server, upload or tracker credential (the F7 rule in
`harness.rs`), and its evidence reaches any consumer only as frozen v5 records plus
`Launch.connector_dir_sha256`. Subscription auth stays in the harness's isolated home and is not
the tracker's.

### Migrating from manifest.toml

The daemon and CLI load `harness.toml` through the shared schema-1 loader, and nothing else.
The V1 product path that used `manifest.toml` was deleted by D48, and the Codex and Claude
`manifest.toml` files and their hook templates by OS-29; the loader refuses a leftover one.
`family`, `tier`, `tested_version_range` and `capture_only` are not harness schema fields (D43
ruling 2).

No schema bump has happened. The [schema evolution rule](CONNECTOR-SCHEMA.md#pin-evolution) says
which widening stays schema 1 and which incompatible change creates schema 2. A schema-bump PR
rewrites every `connectors/*/harness.toml` and regenerates the directory pins; an older build
refuses a newer file with the "newer goalcup" sentence.

## The control API's unauthenticated surface

What a request to the control listener must carry, and which routes need no bearer. The
function is `goalcup_protocol::control::is_public_route`; the daemon and the CLI both call it, so
this section is a reading of it and the function wins.

- **Public routes: one.** `GET /v0/entries/{id}/status` (the signed status document, for an
  id the caller already holds). `GET /v0/ui`, the removed V1 status page, was the second until
  its stub was deleted on 2026-10-12; it now needs the bearer like any unknown path.
  Everything else needs `Authorization: Bearer <token from control.json>`. `/v0/entries/current` and `/v0/entries` are not public: enumeration is not
  accepted.
- **`/v0/assets/*` is not public.** It was the third public prefix until v0.5, and nothing ever
  served it. A request under it is an unauthenticated `401` like any other protected path.
- **A path with a `.` or `..` segment, or a malformed `%` escape, is never public.** The auth
  layer asks about the raw `request.uri().path()`, never a decoded one.
- **The `Host` is checked before any of this.** Exactly one `Host` line, naming the listener's own
  address: `127.0.0.1:<port>`, `localhost:<port>` or `[::1]:<port>`, where `<port>` is the port in
  `control.json` (hostname case-insensitive). A missing, repeated, foreign or differently-ported
  `Host` is refused `401` with code `unauthorized` and a fixed body that quotes nothing it was
  sent, on a public route and on a bearer route alike. This closes DNS rebinding: a page the
  browser believes is `evil.example`, whose name now resolves to `127.0.0.1`, sends
  `Host: evil.example:<port>` and reads nothing. A client that builds its URL from `control.json`
  (the CLI's `HttpDaemonClient`, the test support) sends a matching `Host` without trying.
- **Control API v5.** `CaptureWorkspaceRequest` requires a 64-character lowercase
  `connector_dir_sha256`. The daemon compares it with the authoritative connector it just loaded,
  before capture allocation or writes; mismatch is path-free `422 preflight_failed`. The response
  does not echo it. This control-only bump does not change the frozen v5 chain (D46).
- **Control API v6.** Removes the retired V1 hook-path shapes: `WorkspaceInfo`'s `server_port`,
  `installed_files` and `workspace_relative`; the always-constant `StatusPayload` fields
  `arming_recipe`, `posts_dropped`, `posts_dropped_after_seal` and `pins_dropped`; the
  `arming_timeout` and `unsupported_connector` error codes (an unknown connector is now
  `422 preflight_failed`); and `ProcessHandle.home_seed`, which a v6 `ProcessRegistration`
  refuses as an unknown field. It does not change the frozen v5 chain or its export.
- **Control API v7.** `CaptureWorkspaceRequest` may carry `entry_id`, the id an upload server
  issued (`e` and 16 lowercase hex digits; any other shape is `400 malformed` before anything is
  created); without it the daemon mints one as before. `CaptureWorkspaceInfo` adds `chain_head`
  (the baseline's link hash, which a start receipt signs) and `trace_dir`. It does not change the
  frozen v5 chain or its export.
