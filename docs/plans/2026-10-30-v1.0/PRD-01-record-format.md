# PRD-01: The record format (GCR1)

Id: PRD-01. Version: goalcup 1.0. Status: spec, built first, every other PRD builds against it.
Date: 29 October 2026. Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md).

## Goal

One small binary record format that holds every recorded byte raw, every loss as a gap with a start
and an end, and the counts of what was written. The part file on disk is the exact unit that is
uploaded, stored and fetched.

## Points served

1, 2, 3, 5 and 6. Point 4 through the mask record shape (PRD-05 owns what is masked).

## Owner decisions it carries out

- New record format, the best one: one recording per WebSocket direction, raw bytes, no JSON number
  arrays, no needless base64, light framing, every loss a gap with start and end, written counts,
  off spans (BRIEF3, 29 Oct evening).
- zstd with a long window: `--long=27` made run 1's real trace 4 times smaller (100.8 MB against
  408.9 MB; 128 MiB decompression window).
- No caps of our own. Only stated memory guards; past one, write a gap and go on.
- The file path is the ID for session files; the server orders by ID and time.
- Masking happens before any byte reaches disk (D14).
- Counts: the participant, status, heartbeat and server see only written counts and off time.
- No backward compatibility. Verifier, Worker and client move together.
- Record raw, interpret later: the recorder parses only what it must to relay and to mask.

## Today

| What | Where | Fact |
|---|---|---|
| One JSON envelope per line, `seq`, `prev`, `hash` (64 hex each), body | `crates/goalcup-protocol/src/envelope.rs:9-28` | Every record carries 128 hex characters of chain and a JSON wrapper |
| Link hash | `crates/goalcup-protocol/src/chain.rs:31-36` | `hex(sha256(prev_hex or bytes))` per record, over a canonical JSON re-serialisation |
| Payload as base64 in a `Segment` | `crates/goalcup-protocol/src/capture.rs:280-289` | 33% growth, plus `source_offset`, `source_len`, two times, a 32-hex producer key and an audit per segment |
| Record kinds | `crates/goalcup-protocol/src/capture.rs:256-342` | Open, Segment, End, Gap, Connection, Recovery, ProducerClosed, Launch, Assistance, LocalFinish |
| Gap reasons | `crates/goalcup-protocol/src/capture.rs:191-202` | Gaps carry one time, never an end, and `lost_bytes` was `null` on all 23 gaps of run 1 |
| One stream per WebSocket message | `crates/goalcup-cli/src/proxy/websocket.rs:691-716` (`begin`), `:749-774` (`frame_end`) | 152,728 server-message streams in run 1, three records each |
| Part rotation | `crates/goalcup-protocol/src/export.rs:62-64` | 256 MiB or 30 minutes, uncompressed `chain*.jsonl` |
| Upload compression | `crates/goalcup-cli/src/upload/uploader/chunk.rs:12-19, 144` | zstd level 3, no long window, one frame per chunk, at most 32 MiB compressed |
| Worker | `server/worker/src/chunks.ts:22, 155-158, 199-227` | 32 MiB cap, key `traces/{event}/{run}/{entry}/{offset:016}-{sha256}.zst`, streams to R2 create-only, never decompresses |
| Fetch | `scripts/fetch-run.sh:327` | Decodes each chunk on its own with `zstd -dcq` |

Run 1 numbers (server copy, run report `docs/research/results/2026-10-30-two-hour-run.md`, section
"Size and compression", on branch `origin/smart/tender-clarke-qupne3`): the chain was 1,304,968,017
bytes and 479,320 records. It split into 721.8 MB of payload (55%), 239.3 MB of base64 growth (18%)
and 343.9 MB of record framing (26%). The WebSocket streams took 680.0 MB of chain for 261.7 MB of
payload: 418.3 MB of per-message overhead. Recompressed at `zstd -3` the chain was 408.9 MB; at
`zstd -3 --long=27` it was 100.8 MB (12.9 to 1).

## Decisions at a glance

| Question | Decision | Why, in one line |
|---|---|---|
| Framing | Light binary: `kind`, `length`, body. Raw payload bytes. JSON only for rare metadata | Run 1 lost 344 MB to framing and 239 MB to base64. Binary framing costs about 12 bytes a record |
| Text or binary payload | Raw bytes, never base64, never escaped | Zero growth; long-window matching sees the same bytes at the same alignment every time |
| WebSocket | One stream per direction; one DATA record per message piece, with an end-of-message flag | 152,728 messages cost about 2 MB of framing instead of 418 MB, and message boundaries survive |
| Headers | In the OPEN record's JSON metadata, masked | One place, works for HTTP/1.1 and HTTP/2 alike, rare enough that JSON costs nothing |
| Hash chain | **Keep, but one link per flush (at most one a second), not one per record** | The Worker's continuity check and the shared database's `chunks` columns stay unchanged; torn tails have a clear commit point; the server copy can be proven whole |
| Masks | Length preserving: masked bytes become `*`, one MASK record names the span and the rule | Offsets stay equal to source offsets; JSON and SQLite pages stay readable |
| Compression | zstd level 3, window 2^27, long distance matching, no checksum, written at record time | 4 times smaller on run 1; 2^27 is the largest window `zstd -d` accepts with no flag |
| Unit of upload | The compressed part file itself; a chunk is a byte range between two flush points | No second compression, no spool, and the long window spans the whole part, not one chunk |
| Part size | Rotate at 512 MiB raw or 60 minutes | A part must be larger than the 128 MiB window or the window is wasted |
| Counts | A COUNT event every 60 s and in the seal, of records already written | Counted at write, in the stream itself, so the verifier can recount |
| Clocks | Every record has a monotonic time; every link pairs wall and monotonic time | Clock jumps and sleep are visible and never reorder records |

## 1. Files on disk

One entry (one `goalcup start` or `goalcup continue`) has one folder:

```
$GOALCUP_HOME/runs/<run_id>/<entry_id>/rec/
  rec-00000000000000000000.gcr.zst   part 0: raw offset 0
  rec-00000000000000000000.idx       its flush index
  rec-00000000000536870982.gcr.zst   part 1: starts at raw offset 536,870,982
  rec-00000000000536870982.idx
```

- **The record stream.** Concatenate the raw (decompressed) bytes of every part in name order and you
  get the entry's one record stream. A raw offset is a position in that stream. The name of a part is
  its first raw offset, 20 digits, zero padded.
- **A part file** is one or more complete zstd frames. Concatenated part files are a valid zstd stream,
  so `cat rec-*.gcr.zst | zstd -dc` gives the record stream. Only the last part of a live entry may
  end inside an open frame.
- **The index file** (`.idx`) is local only and never uploaded. One text line per flush point:

  ```
  L <link_seq> <raw_end> <c_end> <link_hash_hex>   a flush point inside an open frame
  E <link_seq> <raw_end> <c_end> <link_hash_hex>   the frame and the part are closed
  R <link_seq> <raw_end> <c_end> <link_hash_hex>   a recovery frame was appended; the part is closed
  ```

  `raw_end` is the raw offset just past the LINK record. `c_end` is the byte length of the part file
  at that point. Every line points at a place where the compressed bytes before it decode to whole
  records ending with a LINK.
- **Rotation.** The writer closes a part when it holds 512 MiB of raw records, or when it has been
  open 60 minutes and holds at least one link. Closing writes a LINK, finishes the frame (zstd writes
  its 3-byte end block `01 00 00`), and only then writes one `E` line. There is never an `L` line for
  the last link of a closed part: that keeps every chunk's raw length above zero.
- **Durability.** The writer calls `fdatasync` on the part, then on the index, at most every 5 s and at
  the seal. The uploader sends only up to the last synced index line (the "durable point", shared in
  memory). After a crash or power loss, what is on disk is durable by definition.
- **Pruning** (PRD-06): a part and its index are deleted once the part is closed and the server's
  offset is at or past its last `raw_end`.
- Other files in the entry folder (`status.json`, upload state) belong to PRD-02, PRD-06 and PRD-10.
  They are not part of the format.

## 2. Framing

Every record is:

```
kind  : u8       which record (table in section 3); 0x00 is never valid
len   : varint   length of body in bytes
body  : len bytes
```

- `varint` is unsigned LEB128, at most 10 bytes for a u64. `zigzag` is the usual signed mapping
  (0, -1, 1, -2 to 0, 1, 2, 3) followed by a varint.
- A reader skips a record of an unknown kind by its length. That is how 1.x adds kinds.
- Bounds a reader enforces, so a corrupt file cannot make it allocate wildly: `len` at most 16 MiB.
  Writers keep DATA payloads at most 256 KiB and OPEN metadata at most 1 MiB.
- JSON bodies are UTF-8, no trailing newline. Readers ignore unknown JSON keys.

**Why binary and not JSON lines.** Run 1 paid 715 bytes of framing per record and base64 on every
payload byte: 583 MB of 1,305 MB was not payload. A JSON line header with a raw body after it
(`{"k":"d","sid":7,"len":678}\n` then the bytes) would remove the base64 and cost about 60 to 100
bytes a record. Binary costs about 12. For run 1's 152,728 WebSocket server messages that is
about 1.8 MB against about 12 MB, and neither escapes or expands a payload byte. The cost of binary
is that a person cannot read a part with `less`. The fix is `goalcup-verify dump` (PRD-08), which
prints one JSON line per record, and a 60-line Python reader in `docs/RECORD-FORMAT.md` (PRD-15).
JSON stays where it is cheap and helps: metadata, events, the seal.

## 3. Records

`sid` is a stream id (section 5). `dt` is the record's monotonic time minus the previous record's
monotonic time, in nanoseconds, as zigzag (section 6). All times are nanoseconds.

| Kind | Name | Body, in order | Notes |
|---|---|---|---|
| 0x01 | PART | `magic` 4 bytes `GCR1`; `ver` u8 = 1; `entry` 17 ASCII bytes (`e` and 16 hex); `boot` varint; `part` varint (0, 1, 2, across boots); `raw_offset` varint; `wall_ns` varint (Unix time); `mono_ns` varint; `boot_id` 16 bytes; `next_link` varint; `prev` 32 bytes | First record of every part. Sets the time base. `prev` is the hash of the last LINK before this part (32 zero bytes for the first part of an entry) |
| 0x02 | OPEN | `sid` varint; `dt` zigzag; `skind` u8 (section 4); `conn` varint (0 if none); `req` varint (0 if none); `meta` JSON (rest of body) | Starts a stream |
| 0x03 | DATA | `sid` varint; `dt` zigzag; `off` varint; `flags` u8; `bytes` (rest of body, 1 byte to 256 KiB) | Masked bytes at stream offset `off`. Time is when the first byte of the piece was read |
| 0x04 | END | `sid` varint; `dt` zigzag; `reason` u8; `seen` varint; optional JSON tail | Ends a stream. `seen` is every source byte the recorder saw for it, written or lost |
| 0x05 | GAP | `sid` varint (0 = all streams); `reason` u8; `start_wall` varint; `end_wall` varint; `start_mono` varint; `end_mono` varint; `off` varint; `lost` varint; optional JSON tail | A loss with a start and an end. `lost` is bytes lost plus one; 0 means unknown. `off` is the stream offset where the loss starts (0 for `sid` 0). The record's time is `end_mono` |
| 0x06 | MASK | `sid` varint; `off` varint; `len` varint; `rule` varint | Bytes `[off, off+len)` of that stream were masked by rule number `rule` (index into the start event's `mask_rules`). Written after the DATA it covers |
| 0x07 | EVENT | `dt` zigzag; JSON (rest of body) with key `ev` | Rare facts: start, launch, exit, count, note, clock, seal (section 9 and 11) |
| 0x08 | LINK | `seq` varint; `wall_ns` varint; `mono_ns` varint; `hash` 32 bytes | A commit and flush point (section 10). The record's time is `mono_ns` |

DATA `flags`:

| Bits | Meaning |
|---|---|
| bit 0 (0x01) | End of WebSocket message (the FIN of the message). Always 0 outside `ws_up` and `ws_down` |
| bits 4 to 7 | WebSocket opcode of the message this piece belongs to: 0x1 text, 0x2 binary, 0x8 close, 0x9 ping, 0xA pong. 0 outside WebSocket streams |
| bits 1 to 3 | Reserved, 0 |

END `reason`: 1 `complete`, 2 `interrupted` (the peer closed mid-message or mid-body), 3
`decode_error` (a body coding failed part way; the decoded prefix is kept), 4 `cut_at_seal` (still
open at the seal), 5 `source_changed` (a session file was replaced or truncated; a new generation
follows). For a `conn` stream the JSON tail is `{"up": <bytes client to server>, "down": <bytes
server to client>}`, counted on the wire, for decrypted and tunnelled connections alike.

GAP `reason` (readers show an unknown number as `other(n)`):

| n | Reason | When |
|---|---|---|
| 1 | `memory_guard` | A stated memory guard was passed (PRD-02). JSON tail names the guard and its value |
| 2 | `recorder_down` | The recorder process died and was restarted. Always `sid` 0. Written by the next boot (section 12) |
| 3 | `unsupported_encoding` | A body coding the proxy cannot decode. No encoded byte is ever written |
| 4 | `decode_failed` | A coding broke part way. The decoded prefix is in DATA |
| 5 | `source_changed` | A session file was truncated, replaced or vanished |
| 6 | `tls_refused` | A listed host's client refused the run CA; that connection or host fell back to a tunnel |
| 7 | `pinned` | A listed host whose client pins certificates; recorded as a tunnel |
| 8 | `write_error` | A disk write failed; written once writing works again |
| 9 | `read_error` | A session file could not be read |
| 10 | `cut_at_seal` | Bytes still unread or unwritten when the seal's time ran out |
| 11 | `h2_unsupported` | An HTTP/2 feature the proxy cannot follow; the rest of that connection is a tunnel |

## 4. Stream kinds

`skind` in OPEN. Every stream is one recorded flow of bytes with an OPEN, DATA records and an END.

| n | Kind | One stream is | `conn` | `req` | `meta` keys (all masked) |
|---|---|---|---|---|---|
| 1 | `conn` | One client connection through the proxy, decrypted or tunnelled. No DATA | 0 | 0 | `host`, `port`, `mode` (`decrypt` or `tunnel`), `listed` (`shared`, `participant`, `config` or null), `reason` (tunnels: `not_listed`, `tls_refused`, `pinned`, `not_tls`, `h2_unsupported`), `alpn`, `tls` (version) |
| 2 | `http_req` | One request body (HTTP/1.1 exchange or HTTP/2 stream). The OPEN carries the head | conn sid | 0 | `method`, `path` (query kept, credential values masked), `ver` (`1.1` or `2`), `h2` (HTTP/2 stream id), `headers` (array of `[name, value]` in wire order; masked values are `*` of the same length), `masked` (header names masked), `coding` (the request's `content-encoding`, or null) |
| 3 | `http_resp` | One final response body. 1xx heads are a `note` event | conn sid | request sid | `status`, `ver`, `h2`, `headers`, `masked`, `coding`, `decoded` (true when DATA is the decoded body) |
| 4 | `ws_up` | Every message the client sent on one WebSocket, in order | conn sid | upgrade request sid | `deflate` (true when permessage-deflate was in use; DATA is always inflated) |
| 5 | `ws_down` | Every message the server sent on that WebSocket | conn sid | upgrade request sid | as `ws_up` |
| 6 | `native` | One generation of one append-only session file | 0 | 0 | `path` (`<source>:<relative path>`), `gen` (0 first), `size_at_open` |
| 7 | `snapshot` | One whole-file copy at one moment (a snapshot form, or a final snapshot of a harness database after the harness exits) | 0 | 0 | `path`, `form` (`snapshot` or `final`), `size`, `mtime_ns` |

Rules:

- **HTTP bodies are de-chunked and decoded** (gzip, deflate, br, zstd, at most one coding) before
  masking. DATA always holds the decoded body. A body that cannot be decoded is a GAP
  (`unsupported_encoding`), never encoded bytes (copied rule: `crates/goalcup-cli/src/proxy/http.rs:18-26`).
- **WebSocket:** one `ws_up` and one `ws_down` per upgraded connection. A message is one or more DATA
  records; the last has the end-of-message flag. Control messages (ping, pong, close) are recorded
  too, with their opcode. Inflated with the connection's deflate state (copied from
  `crates/goalcup-cli/src/proxy/websocket.rs:215-230, 595-604`).
- **Tunnels** are `conn` streams with `mode: "tunnel"`. Their END tail gives bytes each way. A long
  streaming tunnel to an unknown host gets a `note` event (PRD-03 sets the threshold).
- **Session database snapshots:** each of `x.db`, `x.db-wal`, `x.db-shm` is its own `snapshot`
  stream. Masking is length preserving, so pages keep their layout; see Risks for WAL checksums.
- A response with no recorded request still names the request sid if one exists, else `req` is 0.

## 5. IDs

| ID | Form | Scope and use |
|---|---|---|
| Run | `r` and 16 hex (server issued, `server/worker/sql/goal-cup-tracer-upload.sql:93`) | The contest run |
| Entry | `e` and 16 hex (server issued, same file `:133`) | One `start` or `continue`. One record stream |
| Boot | varint in PART, 0 first, plus one per recorder restart in this entry | Which recorder process wrote a record |
| Stream | `sid` varint, from 1, never reused inside a boot; 0 means "no stream" | Global key: `<entry>.<boot>.<sid>`, for example `e0123456789abcdef.0.42` |
| Connection | the `sid` of its `conn` stream | `http_req`, `http_resp`, `ws_up`, `ws_down` name it in `conn` |
| Exchange | the `sid` of the `http_req` | `http_resp` names it in `req`; WebSocket streams name their upgrade request |
| HTTP/2 stream | `meta.h2` | Inside one connection |
| Session file | `meta.path` = `<source>:<relative path>`, plus `meta.gen` | **The path is the ID** across boots and entries. `<source>` is the connector's source name. The path never holds the home folder or the user name |
| Link | `seq` varint from 0 per entry, across boots and parts | The Worker's `first_seq` and `last_seq` |

**Subagent and thread hints.** The recorder adds none. It records raw (AGENTS.md "Record raw,
interpret later"). The hints are already in what it records: request headers (for example Codex's
`thread-id` header on every HTTP POST in run 1), WebSocket client message bodies (Codex's
`client_metadata`), and session file paths (Claude Code's `subagents/agent-*.jsonl`, Codex's one
rollout file per thread). PRD-09 keeps the per-harness table of where each hint lives. A recorder
hint field would duplicate bytes and invite parsing on the hot path.

## 6. Time

- **Monotonic time** is a clock that includes sleep: `CLOCK_BOOTTIME` on Linux, `mach_continuous_time`
  on macOS, in nanoseconds. `CLOCK_MONOTONIC` on Linux stops during suspend, which would shrink a
  laptop's night to nothing (clock_gettime(2)). `boot_id` names that clock: Linux
  `/proc/sys/kernel/random/boot_id`; macOS `kern.bootsessionuuid` (unverified until built).
- **Every record has a monotonic time.** PART, LINK and GAP carry it in full; OPEN, DATA, END and EVENT
  carry `dt` from the previous record in file order. Times can step backwards in file order because
  pieces from many streams meet in one queue; that is why `dt` is signed. The first record after a
  PART takes its `dt` from the PART's `mono_ns`.
- **What the time means.** OPEN: when the head was complete (or the file was found). DATA: when the
  piece's first byte was read from the socket or file. END: when the end was seen. EVENT: when it
  happened. GAP: its own start and end.
- **Wall time.** Every LINK pairs `wall_ns` (`CLOCK_REALTIME`) with `mono_ns`. A reader maps a record's
  monotonic time to wall time with the nearest LINK before it. Links come at least once a second
  while records flow and at least once a minute while the recorder lives (the COUNT event forces one).
- **Clock jumps.** If, between two links, wall time moved more than 2 s away from monotonic time, the
  writer adds `{"ev":"clock","jump_ns":<wall change minus mono change>}`. Records are never reordered
  or re-timed for a jump.
- **Server time.** The start event carries the server's time from the run approval and the local wall
  time when it arrived. The server stamps every chunk with `received_at`. A wrong laptop clock shows
  as a constant skew.
- A machine reboot inside one entry means a new `boot_id`. Monotonic times of different `boot_id`
  values are never compared; a reader uses wall times across them.

## 7. Gaps and off spans

- **Every loss is one GAP with a start, an end, a reason and the lost bytes when known.** A stream gap
  is written when its loss ends: the stream recovers, the stream ends, or the seal comes. The stream
  goes on after it, from `off + lost`.
- **No silent loss.** Every place that drops a byte writes a GAP or adds to an open one. PRD-02
  owns the rule; PRD-08 checks that DATA offsets of every stream are contiguous except where a GAP
  covers the hole.
- **Off spans.** Recording is "off" only while the recorder is not running. That is a GAP with
  `sid` 0 and reason `recorder_down`, from the last durable LINK of the dead boot to the first record
  of the next boot. The sum of `sid` 0 gaps is the off time the participant sees. Every stream of
  the dead boot that has no END is read as cut by that gap.
- **"Losing data now".** While a stream gap is open, the live counters (section 9) already count the
  lost bytes, so `goalcup status` can say so before the GAP record exists.

## 8. Masks

PRD-05 decides which bytes are secret. This PRD fixes how that looks in the record:

- Masked bytes are replaced in place by `*` (0x2A), same length. DATA offsets therefore stay equal to
  source offsets, JSON stays parseable, and a database page keeps its layout.
- Each masked span gets one MASK record (`sid`, `off`, `len`, `rule`). The start event lists the
  rules: `"mask_rules": ["openai_key", "bearer", ...]`. Rule numbers are positions in that list.
- Header and query values in `meta` are masked the same way, and `meta.masked` names the masked
  header names. No MASK record is written for metadata.
- A masked secret is never in any record, gap tail, note or count. Its length is visible. That is
  accepted: the rule name already says what kind of value it was.

## 9. Written counts

A COUNT event is written every 60 s and once more just before the seal. Its values cover only records
**already written before it** in the same boot, so the event is honest by construction and the
verifier recounts it exactly.

```json
{"ev":"count","boot":0,"up_ms":3600000,
 "http_req":812,"http_post":243,"ws_up_msgs":671,"ws_down_msgs":152728,
 "req_whole":905,"req_holes":9,
 "conns":120,"tunnels":118,"tunnel_up":2048000,"tunnel_down":104857600,
 "wire_bytes":674000000,"native_files":43,"native_bytes":48200000,"snapshots":0,
 "gaps":3,"lost_bytes":1500000,"lost_unknown":0,"off_ms":0,
 "open_streams":61,"queue_bytes":1048576,"queue_peak":67108864}
```

- `http_post` plus `ws_up_msgs` (text and binary messages only, on decrypted connections) is the
  **model requests written** number the participant, status, heartbeat (`model_requests`) and server
  see. It is a request count, not a call count. Real calls, models and tokens come from the usage
  reader (PRD-09).
- `req_whole`: those requests whose stream (or message) ended `complete` with no gap. `req_holes`: the
  rest that ended.
- Counts are per boot. The entry total is the sum, over boots, of each boot's last COUNT. The seal
  carries the sum.
- `off_ms` is the length of `recorder_down` gaps written by this boot.

## 10. Links: the hash chain stays, one link per flush

**Decision: keep a hash chain, cut it to one link per flush point.**

```
hash(n) = SHA-256( "GCR1-LINK" || hash(n-1) || covered bytes )
hash(-1) = 32 zero bytes
covered bytes = every raw byte from the end of LINK n-1 (or the start of the entry) to the start of LINK n
```

- The writer writes a LINK, then flushes the compressor, then writes the index line. Links come at
  most once a second while records flow, when 16 MiB of raw records wait, before rotation, and at the
  seal. A PART header is covered by the next link like any record.
- **Why keep it.** (1) The Worker already checks that each chunk continues where the server stands
  (`Goalcup-Prev`, `Goalcup-Head`, `Goalcup-First-Seq`, `server/worker/src/chunks.ts:68-70`), and the
  shared database's `chunks` table has `first_seq`, `last_seq`, `prev`, `head` as required columns
  (`server/worker/sql/goal-cup-tracer-upload.sql:203-214`). Keeping a chain means no website database
  change. (2) A link is the commit point after a crash: everything before it is whole. (3) Run 1's
  server copy was proven whole by its chain (run report, "Whole chain"). PRD-08 does the same.
- **What goes.** The per-record `prev` and `hash` (128 hex characters on 479,320 records), the canonical
  JSON re-serialisation (`chain.rs:9-15`), signed acknowledgements and receipts (cut by BRIEF3).
- **Cost.** One link a second is about 52 bytes, 0.2 MB an hour, about 31 MB in a busy 168-hour
  week. Hashes do not compress. That is under 0.3% of a busy week (section 14).
- What it does not prove: anything against the participant's own machine. Signing is cut; the
  organizers trust the server copy and its `received_at` times.

## 11. The seal

The last records of a sealed entry are:

```
EVENT {"ev":"count", ...}                          final counts of this boot
EVENT {"ev":"seal","reason":"harness_exit","exit":{"code":0},
       "workspace_head":"<40 hex or null>","dirty":"<sha256 hex of git status --porcelain -z>",
       "boots":1,"totals":{ ...COUNT keys summed over boots... }}
LINK
(frame end, idx line E)
```

- `reason` is one of `harness_exit`, `deadline`, `seal_command`, `signal`. `exit` is `{"code":n}`,
  `{"signal":n}` or null.
- Nothing follows the seal's LINK. The final heartbeat commits that link's `seq` and hash as the
  entry's end; the Worker refuses anything past it (`server/worker/src/chunks.ts:60-62`).
- `seal.json`, `baseline.json`, `receipt.json`, `acks.jsonl` and the tunnel list file are gone. The seal,
  the start facts and the tunnels are records.

Other events, all JSON with `ev`:

| `ev` | When | Keys |
|---|---|---|
| `start` | First record after PART, once per boot | `run`, `entry`, `segment`, `goalcup`, `os`, `arch`, `connector`, `connector_sha256`, `harness_version`, `hosts` (`shared`, `participant`, `config` lists in force), `mask_rules`, `deadline`, `server_time`, `local_wall_at_server_time`, `workspace_head`, `resume` (true when boot > 0) |
| `prev_boot` | Boot > 0 | How the previous boot ended, from the watchdog: `{"code":n}` or `{"signal":n}` |
| `launch` | Harness started | `pid`, `program` (file name only) |
| `exit` | Harness ended | `code` or `signal` |
| `count` | Every 60 s, and before the seal | Section 9 |
| `clock` | Wall time jumped | `jump_ns` |
| `note` | Anything a person should see later | `what` (`long_tunnel`, `ca_refused`, `pinned_host`, `informational_response`, `unsupported_coding`), and plain facts, never secret values |
| `seal` | Last | Above |

## 12. Compression, chunks and crash recovery

**Compression.** The writer compresses as it writes: zstd level 3, `window_log` 27, long distance
matching on, content checksum off, no dictionary. Each part is one frame (plus at most one recovery
frame). The compressor is flushed at every LINK, so every index line is a block boundary.

- Memory: about 141 MiB to compress with window 2^27 and long distance matching (measured with
  `zstandard` 0.25 on 300 MiB of synthetic data, appendix M section 4.5). 128 MiB of window to
  decompress.
- Why 27: `zstd -d` accepts windows up to 2^27 with no flag (appendix M section 4.4). A larger window
  would need `--memory` on every reader and about 600 MiB on the participant's machine.
- Why write compressed: the part file is the upload. No spool, no second compressor, and the window
  covers the whole part instead of one chunk. Run 1 at `-3` without a long window was 3.2 to 1
  because the previous body of the same agent fell out of a 2 MiB window when 20 agents interleaved.
- Flushing once a second costs a few bytes per block; the window history is kept across flushes.

**Chunks** (PRD-06 sends them, PRD-07 stores them):

- A chunk is the compressed bytes of one part file from one index line (or the file start) to a later
  index line. It never spans two parts. It is at most 32 MiB (the Worker's cap,
  `server/worker/src/chunks.ts:22`). A flush interval is at most 16 MiB raw, so it always fits.
- Headers keep their names and meaning: `Goalcup-Length` = raw bytes covered (always above zero);
  `Goalcup-First-Seq` = first link seq in it; `Goalcup-Last-Seq` and `Goalcup-Head` = the end line's
  link; `Goalcup-Prev` = the link hash at its start; `Goalcup-Sha256` = SHA-256 of the body; the path
  offset is the raw offset of its first byte.
- A chunk is **not** decodable on its own: it continues its part's frame. That is the price of the long
  window across a whole part. Order and integrity are enforced by offsets, links and the SHA-256 in
  the R2 key.

**What the Worker needs: nothing new.** It stores bytes and never decompresses
(`server/worker/src/chunks.ts:199-227`). `Content-Type: application/zstd` and the key stay. Signing
of acks goes (PRD-07).

**What fetch needs.** List an entry's chunks by offset, check each SHA-256, concatenate in offset order,
then decompress once: `zstd -dc --long=27`. Today's per-chunk decode (`scripts/fetch-run.sh:327`) must
change (PRD-07). An entry still in progress ends inside an open frame: the decoder outputs everything
up to the last chunk and reports the open end, which is expected.

**Crash recovery** (the recorder died; the watchdog starts boot b+1):

1. Read the last part's index. Drop index lines whose `c_end` is past the file's length. Truncate the
   file to the last remaining line's `c_end`. Bytes after it were never uploaded (the uploader sends
   only up to the durable point).
2. If that line is `L`, the frame is open: append `01 00 00`, the 3-byte empty last block that zstd
   itself writes to end a frame after a flush (checked with `zstandard` 0.25: its own end bytes after a
   flush are `010000`, and a frame closed by hand this way, followed by a new frame, decodes as one
   stream).
3. Append one small recovery frame holding `GAP` (`sid` 0, `recorder_down`, start = the last LINK's
   wall and monotonic time, end = now, `lost` 0 = unknown, tail `{"cause": ...}`) and a `LINK`. Write
   an `R` line. This part is now closed, and its next chunk has raw bytes, so the Worker accepts it.
4. Start a new part: PART with `boot` b+1, `prev` = the recovery LINK's hash, then `start` with
   `resume: true`, then `prev_boot`.

If the entry has no index line at all (the recorder died within its first second), delete the empty
part and start boot b+1 at raw offset 0 with a `recorder_down` gap after its `start` event.

## 13. Invariants (PRD-08 checks every one)

1. The stream starts with PART (part 0, boot 0) and every part file starts with PART.
2. Links: `seq` runs 0, 1, 2 without a hole across parts and boots; every hash recomputes.
3. Every DATA, END, GAP (non-zero `sid`) and MASK follows the OPEN of its `sid` in the same boot.
4. Per stream, DATA offsets are contiguous from 0, except where a GAP of that `sid` starts at the
   hole and its known lost bytes fill it exactly. A GAP with unknown loss (`lost` 0) must sit at the
   hole; the hole's size is then reported, not checked.
5. Every MASK lies inside bytes already written for its `sid`, and those bytes are all `*`.
6. Every boot after 0 has exactly one `recorder_down` gap, in the recovery frame just before its
   PART (or, when the entry had no link yet, just after its `start` event).
7. Every COUNT equals a recount of the records before it in its boot.
8. A sealed entry ends with `seal`, then LINK, then a frame end. Nothing follows.
9. Every `http_resp` names a `req` that is an `http_req` on the same `conn`, or 0.

## 14. Size: run 1 under the new format

Payload stays 721.8 MB (the masked bytes are the same length). Base64 (239.3 MB) and per-record
chain framing (343.9 MB) go. What replaces them:

| Piece | Records (est.) | Bytes each (est.) | MB (est.) |
|---|---:|---:|---:|
| WebSocket server messages, one DATA each | 152,728 | 12 | 1.8 |
| WebSocket client messages, DATA pieces of at most 256 KiB | about 1,300 | 12 | under 0.1 |
| HTTP request and response bodies, DATA pieces | about 3,000 | 12 | under 0.1 |
| HTTP OPENs with masked headers in JSON | 1,460 | about 1,000 | 1.5 |
| Connection and WebSocket direction OPENs and ENDs | about 1,000 | about 150 | 0.2 |
| Session file DATA (43 files polled once a second while growing) | about 30,000 | 14 | 0.4 |
| LINKs (at most one a second for 2 hours) | at most 7,200 | 52 | 0.4 |
| COUNT and other events | about 150 | 400 | 0.1 |
| **Framing total** | | | **about 4.5** |

- **Raw:** about 726 MB, against 1,305 MB today (56%).
- **Compressed:** run 1's chain compressed to 100.8 MB at `-3 --long=27`. The new raw stream drops
  the base64 phase problem that cost the long window most of its gain in the synthetic test (14.5 to
  1 with timed cuts against 37.2 to 1 with aligned cuts, appendix M section 4.5). Estimate: 50 to
  100 MB. Plan with 100 MB until measured.
- **A full 2-hour run** (Codex's own record has 1,261 calls against the 842 recorded, so about 1.5
  times the wire): about 1.1 GB raw, about 75 to 150 MB compressed (estimate).
- **A busy week** at run 1's recorded wire rate (674 MB in 49 minutes, about 0.83 GB an hour):
  about 140 GB raw, 10 to 20 GB compressed, about 0.13 to 0.26 Mbit/s of upload on average
  (estimate). R2 at $0.015 per GB-month: $0.15 to $0.30 a month per run.
- **Window reach.** 20 subagents each resending 1.5 MB is about 30 MB between one agent's copies,
  inside the 128 MiB window. At about 85 such agents at once the previous copy falls out of the window
  and the ratio drops toward run 1's 3.2 to 1 (estimate). Dedupe of repeated context is not in 1.0.

## 15. What the usage reader can rely on (PRD-09)

1. Every byte the proxy saw on a decrypted connection is in a DATA record, or inside a GAP with its
   offset, or counted as unknown loss in a GAP. Nothing is dropped without a record.
2. HTTP bodies are decoded (no `content-encoding` left), de-chunked, and masked the same length. JSON
   bodies parse unless a gap cut them; SSE responses are plain text.
3. Every response names its request (`req`), every request names its connection (`conn`), and
   every connection names its host and whether it was on a list (`listed`).
4. WebSocket messages keep their boundaries (end-of-message flag) and their direction (`ws_up`,
   `ws_down`), and are inflated.
5. Session files: `path` and `gen` identify a file; DATA offsets are file offsets; a new generation
   follows a `source_changed` end. Final database snapshots are whole files taken after the harness
   exited.
6. Every record has a monotonic time; every LINK maps it to wall time; the server's `received_at`
   bounds clock skew.
7. COUNT and seal totals are exact recounts of what was written. They are not call counts.
8. The reader must not rely on: a model name, a thread id or a token count being in any fixed place;
   message boundaries in HTTP bodies; masked bytes meaning anything; any order across streams other
   than time.
9. Decoding needs the whole part from its start (128 MiB window, so not inside a Worker's 128 MB
   isolate). Run the reader as an organizer job (PRD-09).

## 16. Worked example

One HTTP/1.1 POST with a streamed reply, then a link. Entry `e0123456789abcdef`, boot 0, part 0, wall
time 2026-11-07T00:00:00Z (1,794,009,600,000,000,000 ns), monotonic time 5,000,000,000 ns, `boot_id`
16 bytes of `0x11`. Computed by a 40-line script; the bytes below are exact.

| Raw offset | Bytes | Record | First bytes (hex) |
|---:|---:|---|---|
| 0 | 90 | PART | `01 58 47 43 52 31 01 65 30 31 32 ...` (kind 1, len 88, `GCR1`, ver 1, `e012...`) |
| 90 | 122 | EVENT `start` | `07 78 00 7b 22 65 76 22 ...` (kind 7, len 120, dt 0, JSON) |
| 212 | 98 | OPEN `conn` sid 1 | `02 60 01 80 89 7a 01 00 00 7b ...` (sid 1, dt +1 ms, kind 1, conn 0, req 0, JSON) |
| 310 | 152 | OPEN `http_req` sid 2 | `02 95 01 02 80 92 f4 01 02 01 00 7b ...` (len 149, sid 2, dt +2 ms, kind 2, conn 1, req 0) |
| 462 | 35 | DATA sid 2 | `03 21 02 80 89 7a 00 00 7b 22 6d 6f ...` (off 0, flags 0, 27 body bytes) |
| 497 | 6 | END sid 2 | `04 04 02 00 01 1b` (dt 0, complete, seen 27) |
| 503 | 127 | OPEN `http_resp` sid 3 | `02 7d 03 80 8c 8d 9e 02 03 01 02 7b ...` (dt +300 ms, kind 3, conn 1, req 2) |
| 630 | 46 | DATA sid 3 | `03 2c 03 80 ad e2 04 00 00 64 61 74 61 ...` (dt +5 ms, off 0, 37 bytes) |
| 676 | 8 | END sid 3 | `04 06 03 80 89 7a 01 25` (dt +1 ms, complete, seen 37) |
| 684 | 49 | LINK seq 0 | `08 2f 00 ...` then `hash` |

The LINK's hash is SHA-256 of the 9 ASCII bytes `GCR1-LINK`, 32 zero bytes and raw bytes 0 to 683:
`3149d18a4f099d0179bb24650a95cb0679c9a0f5a8e57e928b6d76110cebd1fc`. The golden test in
`crates/goalcup-format/tests/golden.rs` pins all 733 bytes and this hash.
Framing is 669 of 733 bytes here because the example has 64 bytes of payload; on real traffic the
same records carry kilobytes to megabytes.

`goalcup-verify dump` prints:

```json
{"o":0,"k":"part","entry":"e0123456789abcdef","boot":0,"part":0,"raw_offset":0,"wall":"2026-11-07T00:00:00Z","mono":5000000000,"next_link":0}
{"o":90,"k":"event","t":5000000000,"ev":"start","run":"r0123456789abcdef","goalcup":"1.0.0","connector":"codex-cli","mask_rules":["openai_key","bearer"]}
{"o":212,"k":"open","sid":1,"t":5001000000,"skind":"conn","meta":{"host":"api.openai.com","port":443,"mode":"decrypt","listed":"shared","alpn":"http/1.1"}}
{"o":310,"k":"open","sid":2,"t":5003000000,"skind":"http_req","conn":1,"meta":{"method":"POST","path":"/v1/responses","ver":"1.1","headers":[["authorization","********************"],["content-type","application/json"]]}}
{"o":462,"k":"data","sid":2,"t":5004000000,"off":0,"len":27,"text":"{\"model\":\"m1\",\"input\":\"hi\"}"}
{"o":497,"k":"end","sid":2,"t":5004000000,"reason":"complete","seen":27}
{"o":503,"k":"open","sid":3,"t":5304000000,"skind":"http_resp","conn":1,"req":2,"meta":{"status":200,"ver":"1.1","headers":[["content-type","text/event-stream"],["content-encoding","br"]],"decoded":true}}
{"o":630,"k":"data","sid":3,"t":5309000000,"off":0,"len":37,"text":"data: {\"type\":\"response.completed\"}\n\n"}
{"o":676,"k":"end","sid":3,"t":5310000000,"reason":"complete","seen":37}
{"o":684,"k":"link","seq":0,"wall":"2026-11-07T00:00:00.310Z","mono":5310000000,"hash":"3149d18a4f099d0179bb24650a95cb0679c9a0f5a8e57e928b6d76110cebd1fc"}
```

The same shape for a WebSocket and a loss (dump lines only, abridged):

```json
{"k":"open","sid":7,"skind":"ws_up","conn":4,"req":6,"meta":{"deflate":true}}
{"k":"open","sid":8,"skind":"ws_down","conn":4,"req":6,"meta":{"deflate":true}}
{"k":"data","sid":7,"off":0,"flags":{"fin":true,"op":"text"},"len":1500000,"note":"6 pieces, the last with fin"}
{"k":"data","sid":8,"off":0,"flags":{"fin":true,"op":"text"},"len":412}
{"k":"gap","sid":7,"reason":"memory_guard","start":"...:00.100Z","end":"...:02.900Z","off":1500000,"lost":1480000,"tail":{"guard":"queue_bytes","limit":536870912}}
{"k":"mask","sid":8,"off":120,"len":51,"rule":"openai_key"}
```

## Files it owns

Create:

- `crates/goalcup-format/Cargo.toml` (deps: `sha2`, `serde_json`, `zstd` with the long window
  setters; nothing async, no TLS)
- `crates/goalcup-format/src/lib.rs` (constants, kinds, reasons, enums)
- `crates/goalcup-format/src/codec.rs` (varint, zigzag, encode and decode of every record)
- `crates/goalcup-format/src/link.rs` (link hash, chain check)
- `crates/goalcup-format/src/part.rs` (`PartWriter`: compressor settings, flush at LINK, index lines,
  rotation, fsync points, recovery steps 1 to 4)
- `crates/goalcup-format/src/read.rs` (streaming reader over parts or fetched chunks; tolerant open end)
- `crates/goalcup-format/src/dump.rs` (one JSON line per record, used by PRD-08)
- `crates/goalcup-format/tests/golden.rs`, `tests/roundtrip.rs`, `tests/recovery.rs`
- `docs/RECORD-FORMAT.md` is written by PRD-15 from this file.

Must not touch: anything under `crates/goalcup/` (PRD-02 and the module owners), `server/`
(PRD-07), `crates/goalcup-verify/` (PRD-08, PRD-09).

## Interfaces with other PRDs

| PRD | Uses | Seam |
|---|---|---|
| PRD-02 | `PartWriter::{open, recover, write(&Record), link(), rotate_if_due(), seal()}`, `Record`, `GapReason`, `EndReason`, `StreamKind`, durable point | The writer thread owns one `PartWriter` |
| PRD-03, PRD-04 | `StreamKind`, meta key names, flag bits | Through PRD-02's recorder handle, never directly |
| PRD-05 | The MASK record and the `*` fill rule | PRD-05 returns spans; PRD-02 writes MASK records |
| PRD-06 | `IndexLine`, `read_index()`, chunk header rules (section 12) | Reads `.idx` and part bytes, never decodes |
| PRD-07 | Chunk headers, key, fetch rule | `scripts/fetch-run.sh` concatenates then decodes |
| PRD-08 | `read`, `dump`, invariants | Every invariant in section 13 is a check |
| PRD-09 | Section 15 | Reads fetched record streams |
| PRD-10 | COUNT keys, gap reasons, seal keys | Participant text names reasons in plain words |
| PRD-17 | Lessons that touch the format (deflate, brotli, h2) | Acceptance cases feed records through `read` |

## What is cut

The v5 envelope, chain and capture contracts: `crates/goalcup-protocol/src/{envelope.rs, chain.rs,
capture.rs, capture_status.rs, export.rs, ack.rs, receipt.rs, seal.rs, provenance.rs, run_secret.rs}`
(about 4,900 src lines) and their tests, deleted with the old crates by PRD-13. No reader for v5 is
kept (no backward compatibility). The v5 corpus fixtures go with them.

## Acceptance

Real-run proofs first (run on the pad, PRD-12):

1. **First real run decodes.** A Claude Code direct-key pad run with one subagent: `cat rec-*.gcr.zst |
   zstd -dc` exits 0; `goalcup-verify check` passes every invariant in section 13; COUNT recounts
   match. Pass: zero invariant failures.
2. **Server copy equals local.** The same run's fetched chunks, concatenated, equal the concatenated
   local part files byte for byte (before pruning). Pass: identical SHA-256.
3. **Kill test.** During a Codex pad run, `kill -9` the recorder twice. Pass: the run finishes; the entry
   decodes; exactly two `recorder_down` gaps with start before end; links verify across all boots;
   the server copy equals local.
4. **Size.** On the first Codex pad run with subagents, report raw bytes, compressed bytes and framing
   share. Pass: framing (all bytes that are not DATA payload) under 2% of raw. If run 1's server copy
   is reachable read-only, a test-only converter (`crates/goalcup-format/tests/tools/`, not shipped)
   rewrites it into GCR1 and reports the same three numbers against section 14.

Automated tests that guard them:

- `golden.rs`: the worked example encodes to the pinned bytes and decodes back; the dump matches.
- `roundtrip.rs`: random records of every kind round trip; unknown kinds are skipped; a body over
  16 MiB is refused; a truncated record at the end is reported, not read.
- `recovery.rs`: a part cut at a random byte recovers to the last index line, closes with `01 00 00`
  and a recovery frame, and the concatenated parts decode as one stream with one `recorder_down` gap;
  `zstd -t` passes on every part.

## Risks and blind spots

- **Binary is opaque to people.** Mitigated by `dump` and the documented Python reader. A participant
  who wants to read their own record needs one of them.
- **A chunk is not decodable alone.** One lost chunk makes the rest of its part unreadable. The Worker
  stores create-only with SHA-256 in the key, and run 1's server copy held every byte; still, a server
  bug now costs up to one part (512 MiB raw), not one chunk.
- **The 141 MiB compressor** sits in the participant's process for the whole run. On a small machine
  PRD-06 may drop to `window_log` 26 (73 MiB); the format allows any window up to 27.
- **Masking with `*` inside a SQLite WAL frame breaks that frame's checksum**, and SQLite then ignores
  that frame and every later one in the WAL. Final snapshots of `-wal` files may read short. PRD-04
  decides (default: record as is and note it; the wire is the primary record).
- **Length-preserving masks show a secret's length.** Accepted.
- **Write-time compression** puts zstd on the writer thread. If it falls behind, PRD-02's memory guard
  writes gaps; it never slows the relay. Peak throughput with long distance matching is unmeasured on
  the target machines.
- **Window reach** with many large-context agents at once (section 14).
- **`kern.bootsessionuuid`** and the zstd crate's long-window setters are unverified until built.
- **One link a second** is about 31 MB a busy week of hashes that do not compress.

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Rotation | 512 MiB raw or 60 minutes | PRD-06 after measuring |
| Link and flush | at most 1 s, or 16 MiB raw | PRD-02 |
| fsync | every 5 s | PRD-02 |
| DATA piece | at most 256 KiB | PRD-02 |
| COUNT | every 60 s | PRD-02 |
| zstd | level 3, window 27, long matching on | PRD-06, with a measured reason |
| Clock jump note | over 2 s | PRD-02 |

## Builder notes

Order of work:

1. `lib.rs` and `codec.rs`: kinds, reasons, varint and zigzag, encode and decode, bounds. Pin the
   worked example as the golden test before anything else. Other builders start from these types, so
   publish them within the first hour, even with `todo!()` in `part.rs`.
2. `link.rs`, then `part.rs` (writer, index, rotation, fsync), then `recovery`.
3. `read.rs` over a directory of parts and over a fetched concatenation; `dump.rs`.

Traps:

- The zstd crate's `Encoder` flushes a block on `Write::flush` and writes the end block on `finish`.
  Set `include_checksum(false)`: a content checksum would make the hand-closed frame in recovery
  invalid. Check that `window_log` and `long_distance_matching` exist on the encoder in the locked zstd
  version before relying on them.
- Never write an `L` line for the last link of a closed part (section 1). Otherwise the uploader can
  send the 3 end bytes alone, with a raw length of 0, which the Worker and the shared database refuse
  (`length > 0`, `server/worker/sql/goal-cup-tracer-upload.sql:206`).
- `dt` is signed. Do not sort records by time when writing.
- Masked meta values keep their length; do not reuse today's `<redacted:kind>` marker.

How a watcher proves it in a real run: after any pad run, run `goalcup-verify check <entry dir>` and
`goalcup-verify check --server <fetched dir>`, then compare the two SHA-256 sums of the concatenated
parts. Kill the recorder once with `kill -9` in one run per night and check acceptance 3.
