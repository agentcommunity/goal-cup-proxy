# PRD-01: The record format (GCR1)

Id: PRD-01. Version: goalcup 1.0. Status: spec, built first; every other PRD builds against it.
Revision 2, 30 October 2026: settles every ask that PRD-02 to PRD-17 made of PRD-01 and the PRD-01 items
of the consistency check and reviews A and B (section "Review answers" at the end). Base: main at
4b226de2. Index: [PRD-00](PRD-00-index.md).
Revision 3, 30 October 2026: alignment fixes against the ledger (section "Alignment answers" at the end).

## Goal

One small binary record format that holds every recorded byte raw, every loss as a gap with a start
and an end, and the counts of what was written. The part file on disk is the exact unit that is
uploaded, stored and fetched.

## Points served

1, 2, 3, 5 and 6. Point 4 through the mask record shape (PRD-05 owns what is masked).

## Owner decisions it carries out

- New record format, the best one: one recording per WebSocket direction, raw bytes, no JSON number
  arrays, no needless base64, light framing, every loss a gap with start and end, written counts,
  off spans (BRIEF3, 29 Oct evening; ledger F1).
- zstd with a long window: `--long=27` made run 1's real trace 4 times smaller (100.8 MB against
  408.9 MB; 128 MiB decompression window) (ledger F2).
- No caps of our own. Only stated memory guards; past one, write a gap and go on (ledger R5).
- No silent loss: every loss is a gap with a start, an end, a reason and lost bytes where known (R4).
- The file path is the ID for session files; the server orders by ID and time (H8).
- Masking happens before any byte reaches disk (D14, ledger M1).
- Counts: the participant, status, heartbeat and server see only written counts and off time (R6).
- No backward compatibility. Verifier, Worker and client move together (L7, S1).
- Record raw, interpret later: the recorder parses only what it must to relay and to mask (R8).

## Today

| What | Where | Fact |
|---|---|---|
| One JSON envelope per line, `seq`, `prev`, `hash` (64 hex each), body | `crates/goalcup-protocol/src/envelope.rs:9-28` | Every record carries 128 hex characters of chain and a JSON wrapper |
| Link hash | `crates/goalcup-protocol/src/chain.rs:31-36` | `hex(sha256(prev_hex or bytes))` per record, over a canonical JSON re-serialisation |
| Payload as base64 in a `Segment` | `crates/goalcup-protocol/src/capture.rs:280-289` | 33% growth, plus `source_offset`, `source_len`, two times, a 32-hex producer key and an audit per segment |
| Record kinds | `crates/goalcup-protocol/src/capture.rs:256-342` | Open, Segment, End, Gap, Connection, Recovery, ProducerClosed, Launch, Assistance, LocalFinish |
| Gap reasons | `crates/goalcup-protocol/src/capture.rs:191-202` (the reasons), `:295-300` (the `Gap` variant) | Gaps carry one time, never an end, and `lost_bytes` was `null` on all 23 gaps of run 1 |
| One stream per WebSocket message | `crates/goalcup-cli/src/proxy/websocket.rs:691-716` (`begin`), `:749-774` (`frame_end`) | 152,728 server-message streams in run 1, three records each |
| Part rotation | `crates/goalcup-protocol/src/export.rs:62-64` | 256 MiB or 30 minutes, uncompressed `chain*.jsonl` |
| Upload compression | `crates/goalcup-cli/src/upload/uploader/chunk.rs:12-19, 144` | zstd level 3, no long window, one frame per chunk, at most 32 MiB compressed |
| Worker | `server/worker/src/chunks.ts:22, 155-158, 199-227` | 32 MiB cap, key `traces/{event}/{run}/{entry}/{offset:016}-{sha256}.zst`, streams to R2 create-only, never decompresses |
| Fetch | `scripts/fetch-run.sh:327` | Decodes each chunk on its own with `zstd -dcq` |

Run 1 numbers (server copy, run report `docs/research/results/2026-10-30-two-hour-run.md`, section
"Size and compression", on branch `origin/smart/tender-clarke-qupne3`; PRD-15 copies it to main): the
chain was 1,304,968,017 bytes and 479,320 records. It split into 721.8 MB of payload (55%), 239.3 MB
of base64 growth (18%) and 343.9 MB of record framing (26%). The WebSocket streams took 680.0 MB of
chain for 261.7 MB of payload: 418.3 MB of per-message overhead. Recompressed at `zstd -3` the chain
was 408.9 MB; at `zstd -3 --long=27` it was 100.8 MB (12.9 to 1).

## Decisions at a glance

| Question | Decision | Why, in one line |
|---|---|---|
| Framing | Light binary: `kind`, `length`, body. Raw payload bytes. JSON only for rare metadata | Run 1 lost 344 MB to framing and 239 MB to base64. Binary framing costs about 12 bytes a record |
| Text or binary payload | Raw bytes, never base64, never escaped | Zero growth; long-window matching sees the same bytes at the same alignment every time |
| WebSocket | One stream per direction; one DATA record per message piece, with an end-of-message flag | 152,728 messages cost about 2 MB of framing instead of 418 MB, and message boundaries survive |
| Headers | In the OPEN record's JSON metadata, masked | One place, works for HTTP/1.1 and HTTP/2 alike, rare enough that JSON costs nothing |
| Hash chain | **Keep, but one link per flush (about one a second, section 10), not one per record.** A link hashes everything since the last link, its own fields included | The Worker's continuity check and the shared database's `chunks` columns stay unchanged; torn tails have a clear commit point; the server copy can be proven whole |
| Masks | Length preserving: masked bytes become `*`, one MASK record names the span and the rule | Offsets stay equal to source offsets; JSON and SQLite pages stay readable |
| Compression | zstd level 3, window 2^27, long distance matching (window and long matching locked, ledger F2), no checksum, no content size, written at record time | 4 times smaller on run 1; 2^27 is the largest window `zstd -d` accepts with no flag |
| Unit of upload | The compressed part file itself; a chunk is a byte range between two flush points | No second compression, no spool, and the long window spans the whole part file, not one chunk |
| Part file size | Rotate at 512 MiB raw or 60 minutes | A part file must be larger than the 128 MiB window or the window is wasted |
| Counts | A COUNT event every 60 s and in the seal, of records already written | Counted at write, in the stream itself, so the verifier can recount |
| Clocks | Every record has a time on a clock that includes sleep; every link pairs wall and that time; suspend is written down | Clock jumps and sleep are visible and never reorder records |

Words: a **part file** is one `rec-*.gcr.zst` file. The participant's word "part" means an entry
(one `start` or `continue`, PRD-10); this PRD never uses "part" alone for a file.

## 1. Files on disk

One entry (one `goalcup start` or `goalcup continue`) has one folder:

```
$GOALCUP_HOME/runs/<run_id>/<entry_id>/rec/
  rec-00000000000000000000.gcr.zst   part file 0: raw offset 0
  rec-00000000000000000000.idx       its flush index
  rec-00000000000536870982.gcr.zst   part file 1: starts at raw offset 536,870,982
  rec-00000000000536870982.idx
```

- **The record stream.** Concatenate the raw (decompressed) bytes of every part file in name order and
  you get the entry's one record stream. A raw offset is a position in that stream. The name of a part
  file is its first raw offset, 20 digits, zero padded.
- **A part file** is one or more complete zstd frames. Concatenated part files are a valid zstd stream,
  so `cat rec-*.gcr.zst | zstd -dc` gives the record stream. Only the last part file of a live entry may
  end inside an open frame.
- **The index file** (`.idx`) is local only and never uploaded. One text line per flush point:

  ```
  L <link_seq> <raw_end> <c_end> <link_hash_hex>   a flush point inside an open frame
  E <link_seq> <raw_end> <c_end> <link_hash_hex>   the frame and the part file are closed
  R <link_seq> <raw_end> <c_end> <link_hash_hex>   a recovery frame was appended; the part file is closed
  ```

  `raw_end` is the raw offset just past the LINK record. `c_end` is the byte length of the part file
  at that point. Every line points at a place where the compressed bytes before it decode to whole
  records ending with a LINK.
- **Write order.** An index line is written only after every compressed byte up to its `c_end` has
  been written to the part file. A reader takes only index lines that end in a newline; a last line
  without one is ignored.
- **Rotation.** The writer closes a part file when it holds 512 MiB of raw records, or when it has been
  open 60 minutes and holds at least one link. Closing writes a LINK, finishes the frame (zstd writes
  its 3-byte end block `01 00 00`), and only then writes one `E` line. There is never an `L` line for
  the last link of a closed part file: that keeps every chunk's raw length above zero. Then, in this
  order: `fdatasync` the closed part file, `fdatasync` its index, publish the durable point at the `E`
  line, drop the old compressor, and only then open the next part file and allocate its compressor
  (no moment with two 141 MiB compressors).
- **Durability.** The writer calls `fdatasync` on the part file, then on the index, at most every 5 s,
  at every rotation and at the seal. The uploader sends only up to the last synced index line (the
  "durable point", shared in memory). After a crash or power loss, what is on disk is durable by
  definition. At each fsync the durable index length (part file name and index byte length) is
  recorded in `cursors.json` (PRD-02, atomic rename). When recovery runs under a new `boot_id` (the
  machine rebooted), it first drops index lines past that length (section 12).
- **Pruning** (PRD-06): a part file and its index are deleted once the part file is closed, the
  server's offset is at or past its last `raw_end`, and a later part file has an index line. So the
  last part file with an index line is always on disk for crash recovery (section 12).

**Only part files are uploaded.** Every other file is local, never uploaded, and not part of the format.
PRD-01 fixes only that rule; the owners fix the content:

| File | Where | Owner |
|---|---|---|
| `rec/*.idx`, `rec/lock`, `rec/watchdog.pid` | entry folder | PRD-01 (index), PRD-02 |
| `status.json`, `counts.jsonl`, `cursors.json` | entry folder | PRD-02 |
| `upload.json`, `upload.lock` | entry folder | PRD-06 |
| `goalcup.log` | entry folder | PRD-10 |
| `run-ca.pem` (certificate only), `bundle.pem` | entry folder | PRD-03 (code), written by PRD-02's watchdog |
| `rec/run-ca.key` (the run CA key, 0600, from the mint to the seal, for `goalcup recover`; ledger R3b; never uploaded) | entry folder | PRD-02 |
| `server.json` | run folder `$GOALCUP_HOME/runs/<run_id>/` | PRD-10 |

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
  Writers keep DATA payloads at most 256 KiB and OPEN metadata and EVENT bodies at most 1 MiB.
- JSON bodies are compact UTF-8 (no spaces, no trailing newline). Key order carries no meaning; the
  writer may use any order, and the golden files use keys sorted by byte value (what `serde_json`
  gives for a `Value` map). Readers ignore unknown JSON keys.
- An optional JSON tail (END, GAP) is the rest of the body after the fixed fields. No tail means no
  bytes, not `{}`.

**Why binary and not JSON lines.** Run 1 paid 715 bytes of framing per record and base64 on every
payload byte: 583 MB of 1,305 MB was not payload. A JSON line header with a raw body after it
(`{"k":"d","sid":7,"len":678}\n` then the bytes) would remove the base64 and cost about 60 to 100
bytes a record. Binary costs about 12. For run 1's 152,728 WebSocket server messages that is
about 1.8 MB against about 12 MB, and neither escapes or expands a payload byte. The cost of binary
is that a person cannot read a part file with `less`. The fix is `goalcup-verify dump` (PRD-08), which
prints one JSON line per record, and a 60-line Python reader in `docs/RECORD-FORMAT.md` (PRD-15) that
reads the already decompressed stream on standard input (`zstd -dc ... | python3 gcr.py`), so it needs
no third-party zstd package. JSON stays where it is cheap and helps: metadata, events, the seal.

## 3. Records

`sid` is a stream id (section 5). `dt` is the record's time minus the previous record's time, in
nanoseconds, as zigzag (section 6). All times are nanoseconds.

| Kind | Name | Body, in order | Notes |
|---|---|---|---|
| 0x01 | PART | `magic` 4 bytes `GCR1`; `ver` u8 = 1; `entry` 17 ASCII bytes (`e` and 16 hex); `boot` varint; `part` varint (0, 1, 2, across boots); `raw_offset` varint; `wall_ns` varint (Unix time); `mono_ns` varint; `boot_id` 16 bytes; `next_link` varint; `prev` 32 bytes | First record of every part file. Sets the time base. `prev` is the hash of the last LINK before this part file (32 zero bytes for the first part file of an entry) |
| 0x02 | OPEN | `sid` varint; `dt` zigzag; `skind` u8 (section 4); `conn` varint (0 if none); `req` varint (0 if none); `meta` JSON (rest of body) | Starts a stream |
| 0x03 | DATA | `sid` varint; `dt` zigzag; `off` varint; `flags` u8; `bytes` (rest of body, 1 byte to 256 KiB) | Masked bytes at stream offset `off`. Time is when the first byte of the piece was read |
| 0x04 | END | `sid` varint; `dt` zigzag; `reason` u8; `seen` varint; optional JSON tail | Ends a stream. `seen` is the stream offset just past the last source byte the recorder saw for it, written or lost: `from` (section 4) plus every byte seen. For a `conn` stream it is 0 |
| 0x05 | GAP | `sid` varint (0 = all streams); `reason` u8; `start_wall` varint; `end_wall` varint; `start_mono` varint; `end_mono` varint; `off` varint; `lost` varint; optional JSON tail | A loss with a start and an end. `lost` is bytes lost plus one; 0 means unknown, 1 means none. `off` is the stream offset where the loss starts; it is 0 for `sid` 0 and for a `conn` stream. The record's time is `end_mono` |
| 0x06 | MASK | `sid` varint; `off` varint; `len` varint; `rule` varint | Bytes `[off, off+len)` of that stream were masked by rule number `rule` (section 8). Written after the DATA it covers. Carries no time (section 6) |
| 0x07 | EVENT | `dt` zigzag; JSON (rest of body) with key `ev` | Rare facts: start, launch, exit, count, note, clock, suspend, seal (sections 6, 9, 11) |
| 0x08 | LINK | `seq` varint; `wall_ns` varint; `mono_ns` varint; `hash` 32 bytes | A commit and flush point (section 10). The record's time is `mono_ns` |

DATA `flags`:

| Bits | Meaning |
|---|---|
| bit 0 (0x01) | End of WebSocket message (the FIN of the message). Always 0 outside `ws_up` and `ws_down` |
| bits 4 to 7 | WebSocket opcode of the message this piece belongs to: 0x1 text, 0x2 binary, 0x8 close, 0x9 ping, 0xA pong. 0 outside WebSocket streams |
| bits 1 to 3 | Reserved, 0 |

END `reason`: 1 `complete`, 2 `interrupted` (the peer closed mid-message or mid-body), 3
`decode_error` (a body coding failed part way; the decoded prefix is kept), 4 `cut_at_seal` (still
open at the seal), 5 `source_changed` (a session file was replaced, truncated or vanished; a new
generation may follow). A `source_changed` END needs no GAP when the reader had read every byte it
had seen before the change; bytes written in the last interval and never seen are unknowable.

END tail keys (all optional, all counts or masked text, never a secret value):

| Stream kind | Keys |
|---|---|
| `conn` | `up`, `down`: bytes each way on the TCP wire (TLS records included for `decrypt`). `plain_up`, `plain_down`: plaintext bytes each way, `decrypt` mode only |
| `http_req`, `http_resp` | `wire`: body bytes on the wire before de-chunking and decoding (`seen` counts decoded bytes). `trailers`: masked header list, as `headers`. `h2_error`: the `RST_STREAM` or `GOAWAY` error code. `unsent` (`http_req` only): bytes read from the client and recorded but never forwarded (the upstream had closed) |
| `ws_up`, `ws_down` | `close_code`: the close frame's code. `unsent` (`ws_up` only), as above |

GAP `reason` (readers show an unknown number as `other(n)`):

| n | Reason | When | Written by |
|---|---|---|---|
| 1 | `memory_guard` | A stated memory guard was passed. Tail: `{"guard": <name>, "limit": <bytes>}` with a name from the guard list below. Also the close GAP of a tunnel to a listed host whose ClientHello passed the `client_hello` guard (`conn` reason `hello_too_big`, section 4) | PRD-02, PRD-03 |
| 2 | `recorder_down` | The recorder was not running: it died, or ran in null mode. Always `sid` 0. Written by the next boot or by the recovery of a dead entry (section 12). Tail: `{"cause": ..., "prev": {...}}` | PRD-02 |
| 3 | `unsupported_encoding` | A body coding the proxy cannot decode. No encoded byte is ever written. Tail: `{"codings": [...]}` (the coding tokens as sent, each at most 32 bytes, at most 4) | PRD-03 |
| 4 | `decode_failed` | A coding or the framing broke part way. The decoded prefix is in DATA | PRD-03 |
| 5 | `source_changed` | A session file was truncated, replaced or vanished with bytes still unread. Tail may hold `unread_at_least` (bytes) when `lost` is 0 | PRD-04 |
| 6 | `tls_refused` | A listed host's client refused the run CA; that connection fell back to a tunnel | PRD-03 |
| 7 | `pinned` | A listed host whose client pins certificates; recorded as a tunnel | PRD-03 |
| 8 | `write_error` | **Reserved, never written by 1.0.** A write error ends the recorder (exit 74); the next `recorder_down` gap names it in `cause` | none |
| 9 | `read_error` | **Reserved, never written by 1.0.** A failed read is retried; a file that ends unread is `source_changed` | none |
| 10 | `cut_at_seal` | Bytes still unread or unwritten when the seal's time ran out | PRD-02, PRD-04 |
| 11 | `h2_unsupported` | An HTTP/2 feature or frame the proxy cannot follow; the rest of that connection relays unparsed. Never a tunnel reason: the connection is already decrypted | PRD-03 |
| 12 | `mask_failed` | The masker failed on a piece; the piece was dropped so no raw byte was written. `lost` is its length plus one | PRD-02 |
| 13 | `upstream_untrusted` | A listed host whose server certificate fails the recorder's check, or that asks for a client certificate; tunnelled | PRD-03 |
| 14 | `protocol_other` | A listed host reached with a protocol the proxy does not parse (ALPN other than `h2` and `http/1.1`, or not TLS and not HTTP); tunnelled | PRD-03 |
| 15 | `after_stop` | Bytes relayed after the seal began and recording had stopped (helpers and children that outlive the harness). `sid` 0. Tail: `{"conns": n, "hosts": [...]}` | PRD-02 |

`lost` is counted in the stream's offset space: decoded bytes for HTTP bodies, inflated bytes for
WebSocket, file bytes for session files. When that size is unknown, as for `unsupported_encoding` and a
body not decoded under `decoder_window`, `lost` is 0 and the tail adds `{"wire": <encoded bytes>}`.

Memory guard names (the `guard` key): `held_bytes` (PRD-02's pipeline guard), `client_hello`,
`http_head`, `upgrade_copy`, `hpack_table`, and `decoder_window` if PRD-03 counts decoder memory
(review A finding 10). A new guard adds a name here; readers show an unknown name as is.

## 4. Stream kinds

`skind` in OPEN. Every stream is one recorded flow of bytes with an OPEN, DATA records and an END.

| n | Kind | One stream is | `conn` | `req` | `meta` keys (all masked) |
|---|---|---|---|---|---|
| 1 | `conn` | One client connection through the proxy, decrypted or tunnelled. No DATA | 0 | 0 | `host`, `port`, `mode` (`decrypt` or `tunnel`), `listed` (`shared`, `participant`, `config` or null), `kind` (`provider`, `gateway`, `vendor`, `other`, or null when not listed), `reason` (tunnels: `not_listed`, `sign_in`, `tls_refused`, `pinned`, `upstream_untrusted`, `alpn_other`, `not_tls`, `hello_too_big`; null when decrypted), `alpn`, `tls` (version, or null for a listed host spoken to in plain HTTP after `CONNECT`), `fingerprint` (first 16 hex of the ClientHello fingerprint, or null) |
| 2 | `http_req` | One request body (HTTP/1.1 exchange or HTTP/2 stream). The OPEN carries the head | conn sid | 0 | `method`, `path` (query dropped, as in 0.7; URL userinfo masked), `ver` (`1.1` or `2`), `h2` (HTTP/2 stream id; absent for 1.1), `headers` (array of `[name, value]` in wire order; for HTTP/2 the pseudo-headers `:method`, `:path`, `:authority`, `:scheme` are included where they stand, and `:path` has its query dropped like `path`; masked values are `*` of the same length), `masked` (every header name with any masked byte), `coding` (list of codings undone, in order, from `content-encoding` and transfer codings other than `chunked`, or null) |
| 3 | `http_resp` | One final response body. 1xx heads are a `note` event | conn sid | request sid | `status`, `ver`, `h2`, `headers` (HTTP/2: `:status` included), `masked`, `coding`, `decoded` (true when DATA is the decoded body) |
| 4 | `ws_up` | Every message the client sent on one WebSocket, in order | conn sid | upgrade request sid | `deflate` (true when permessage-deflate was in use; DATA is always inflated) |
| 5 | `ws_down` | Every message the server sent on that WebSocket | conn sid | upgrade request sid | as `ws_up` |
| 6 | `native` | One generation of one append-only session file, from one boot | 0 | 0 | `path` (`<source>:<relative path>`), `gen` (0 first), `size_at_open`, `from` (first offset; 0 when absent), `fid` (file identity: device, inode and on macOS birth time, as one short hex string) |
| 7 | `snapshot` | One whole-file copy at one moment | 0 | 0 | `path`, `form` (`snapshot` for a small file copied when it settles; `final` for one file of a database copy), `size`, `mtime_ns`, `fid`, `set` (form `final`: `"<boot>.<n>"`, the same for every file of one copy), `harness_exited` (form `final`: false when the harness still ran) |

Rules:

- **HTTP bodies are de-chunked and decoded** (gzip, deflate, br, zstd, and transfer codings other than
  `chunked`) before masking. DATA always holds the decoded body; `coding` lists what was undone. A body
  that cannot be decoded is a GAP (`unsupported_encoding`), never encoded bytes. This extends the
  copied rule (`crates/goalcup-cli/src/proxy/http.rs:18-26` undoes at most one coding): every coding in
  `content-encoding` is undone in order; any coding not in the list is a GAP `unsupported_encoding`.
- **WebSocket:** one `ws_up` and one `ws_down` per upgraded connection. A message is one or more DATA
  records; the last has the end-of-message flag. Control messages (ping, pong, close) are recorded
  too, with their opcode. Inflated with the connection's deflate state (copied from
  `crates/goalcup-cli/src/proxy/websocket.rs:215-230, 595-604`). Offsets run over the inflated bytes of
  every message of that direction.
- **Tunnels** are `conn` streams with `mode: "tunnel"`. Their END tail gives bytes each way. A tunnel to
  a listed host is a loss, so it also gets one GAP on its `conn` stream, written at close, with the
  GAP reason that matches its `reason` (`tls_refused` 6, `pinned` 7, `upstream_untrusted` 13,
  `alpn_other` and `not_tls` 14, `hello_too_big` 1 with tail `{"guard":"client_hello","limit":65536}`),
  `off` 0 and `lost` = `up` + `down` + 1. A
  tunnel to an unlisted host is not a loss and gets no GAP; a long streaming one gets a `note` event
  (PRD-03 sets the threshold).
- **A decrypted connection that stops parsing** (HTTP/1.1 framing broke, an HTTP/2 frame the proxy
  cannot follow, the upgrade copy guard) relays the rest unparsed. The open stream gets its own GAP,
  and the `conn` stream gets one GAP from the fault to the close with reason `decode_failed`,
  `h2_unsupported` or `memory_guard`, `off` 0 and `lost` = plaintext bytes relayed after the fault + 1.
  So a request made later on that connection is never silent (review A finding 2).
- **Session database copies:** each of `x.db`, `x.db-wal`, `x.db-shm` is its own `snapshot` stream with
  `form: "final"` and one shared `set`. `-shm` is copied when a row matches it but carries nothing a
  reader needs. A copy may be taken at the seal or, if PRD-04 decides so, during the run; `harness_exited`
  says which. Masking is length preserving, so pages keep their layout; repairing WAL frame checksums
  before reading is the reader's job (PRD-09, decided in PRD-04 section 6).
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
| Session file | `meta.path` = `<source>:<relative path>`, plus `meta.gen` | **The path is the ID** across boots and entries. `<source>` is the connector's source name. The path never holds the source's root folder. A harness's own folder names below the root may hold parts of the workspace path (Claude Code names project folders after the working folder, for example `-Users-alice-goalcup-run`, `docs/plans/2026-10-30-audit/O-harness-access.md:582`); the same text is inside the transcripts anyway |
| Database copy | `meta.set` | The files of one SQLite copy |
| Link | `seq` varint from 0 per entry, across boots and part files | The Worker's `first_seq` and `last_seq` |

**Subagent and thread hints.** The recorder adds none. It records raw (AGENTS.md "Record raw,
interpret later"). The hints are already in what it records: request headers (for example Codex's
`thread-id` header on every HTTP POST in run 1), WebSocket client message bodies (Codex's
`client_metadata`), and session file paths (Claude Code's `subagents/agent-*.jsonl`, Codex's one
rollout file per thread). PRD-09 keeps the per-harness table of where each hint lives. A recorder
hint field would duplicate bytes and invite parsing on the hot path.

## 6. Time

- **The record clock** is a monotonic clock that includes sleep: `CLOCK_BOOTTIME` on Linux,
  `mach_continuous_time` on macOS, in nanoseconds. `CLOCK_MONOTONIC` on Linux stops during suspend,
  which would shrink a laptop's night to nothing (clock_gettime(2)). `boot_id` names that clock: Linux
  `/proc/sys/kernel/random/boot_id`; macOS `kern.bootsessionuuid` (unverified until built). Every timer
  in the recorder and the watchdog uses this clock (PRD-02, PRD-06).
- **Every record but MASK has a time.** PART, LINK and GAP carry it in full; OPEN, DATA, END and EVENT
  carry `dt` from the previous timed record in file order. MASK carries no time and does not move the
  running time: the next record's `dt` is from the record before the MASK. Times can step backwards in
  file order because pieces from many streams meet in one queue; that is why `dt` is signed. The first
  record after a PART takes its `dt` from the PART's `mono_ns`.
- **What the time means.** OPEN: when the head was complete (or the file was found). DATA: when the
  piece's first byte was read from the socket or file. END: when the end was seen. EVENT: when it
  happened. GAP: its own start and end.
- **Wall time.** Every LINK pairs `wall_ns` (`CLOCK_REALTIME`) with `mono_ns`. A reader maps a record's
  time to wall time with the nearest LINK before it. When links are written is set once, in section 10.
- **Clock jumps.** If, between two links, wall time moved more than 2 s away from the record clock, the
  writer adds `{"ev":"clock","jump_ns":<wall change minus record clock change>}`. Sleep moves both
  clocks together, so it is never a jump. Records are never reordered or re-timed for a jump.
- **Suspend.** At each link the writer also reads the awake clock (`CLOCK_MONOTONIC` on Linux,
  `mach_absolute_time` on macOS; both stop during sleep). If the record clock moved more than 2 s more
  than the awake clock since the last link, it adds `{"ev":"suspend","ns":<the difference>}`. Sleep is
  not off time (PRD-10); this event tells readers and the verifier why a boot was quiet (invariant 13).
- **Server time.** The start event carries the server's time from the run approval and the local wall
  time when it arrived. The server stamps every chunk with `received_at`. A wrong laptop clock shows
  as a constant skew.
- A machine reboot inside one entry means a new `boot_id`. Record clock times of different `boot_id`
  values are never compared; a reader uses wall times across them. A GAP whose end was taken under a
  different `boot_id` than its start carries tail key `"boot_id"` (the new clock's id, 32 hex). For such
  a GAP, invariant 12 and off time use `start_wall` and `end_wall`.

## 7. Gaps and off spans

- **Every loss is one GAP with a start, an end, a reason and the lost bytes when known.** A stream gap
  is written when its loss ends: the stream recovers, the stream ends, or the seal comes. The stream
  goes on after it, from `off + lost - 1` when the loss is known.
- **A GAP has an extent.** `end_mono` is after `start_mono`, or `lost` is above 1. A GAP with start equal
  to end and no known bytes (run 1's shape) is invalid (invariant 12).
- **No silent loss.** Every place that drops a byte writes a GAP or adds to an open one. PRD-02
  owns the rule; PRD-08 checks that DATA offsets of every stream are contiguous except where a GAP
  covers the hole, and that `seen` agrees.
- **Off spans.** Recording is "off" only while the recorder is not running or runs in null mode. That
  is a GAP with `sid` 0 and reason `recorder_down`, from the last LINK named by an index line (the
  point recovery truncates to, section 12 step 1; after a plain process kill at most about 1 s before
  the kill) to the first record of the next boot (or to the recovery of a dead entry). The sum of
  `recorder_down` gaps is the off time the participant sees (section 9's entry total). Every stream of
  the dead boot that has no END is read as cut by that gap.
- **After the stop.** Bytes relayed after the seal began are not recorded; when there were any, the seal
  writes one `after_stop` GAP that counts them (section 11). It is not off time.
- **"Losing data now".** While a stream gap is open, the live counters already count the lost bytes,
  so `goalcup status` can say so before the GAP record exists.

## 8. Masks

PRD-05 decides which bytes are secret. This PRD fixes how that looks in the record:

- Masked bytes are replaced in place by `*` (0x2A), same length. DATA offsets therefore stay equal to
  source offsets, JSON stays parseable, and a database page keeps its layout.
- Each masked span gets a MASK record (`sid`, `off`, `len`, `rule`). A span that crosses DATA records
  is written as several MASK records, one per DATA record it touches.
- `rule` indexes the start event's `mask_rules` list, which names **kinds**, not literals:
  `"mask_rules": ["openai_key", "bearer", ..., "fail_closed"]`. Two literals of one kind share a
  number. `fail_closed` is the kind for bytes masked because the masker's hold overflowed and it could
  not decide (PRD-05); COUNT counts those bytes apart (`mask_failclosed_bytes`, section 9), so a
  stream that is all stars is visible (review A finding 3).
- PRD-05 promises that an unescaped `"` is never inside a MASK span, so JSON stays parseable, except a
  `fail_closed` span, which may cover quotes; such a body is not expected to parse.
- Header values and URL userinfo in `meta` are masked the same way (the query is dropped, section 4),
  and `meta.masked` names every header with any masked byte. No MASK record is written for metadata.
- Exact values (PRD-05) are announced by counts only: the start event's `mask_values` is
  `{"<source>": n, ..., "skipped": n}`, never a value, a hash or a length of a value. Values added
  during the run, if PRD-05 adds any, are announced by a `note` with `what: "mask_values"` and counts.
- A masked secret is never in any record, gap tail, note or count. Its length is visible. That is a
  planner default (PRD-05), listed in PRD-00's defaults for the owner: the rule name already says what
  kind of value it was.

## 9. Written counts

A COUNT event is written every 60 s, once more just before the seal, and once in each recovery frame
for the dead boot (section 12). Its values cover only records **already written before it** in the same
boot, so the event is honest by construction and the verifier recounts it exactly. A record belongs to
the boot of the nearest PART before it, so a recovery frame belongs to the dead boot.

```json
{"ev":"count","boot":0,"up_ms":3600000,
 "http_req":812,"http_post":243,"ws_up_msgs":671,"ws_down_msgs":152728,
 "req_whole":905,"req_holes":9,
 "conns":120,"tunnels":118,"tunnel_up":2048000,"tunnel_down":104857600,
 "wire_bytes":674000000,"native_files":43,"native_bytes":48200000,"snapshots":0,
 "gaps":3,"lost_bytes":1500000,"lost_unknown":0,"off_ms":0,"mask_failclosed_bytes":0,
 "hosts":{"api.openai.com":914},
 "tunnel_hosts":{"github.com":{"conns":12,"up":40960,"down":2097152,"listed":null}},
 "open_streams":61,"held_bytes":1048576,"held_peak":67108864}
```

- `http_post` plus `ws_up_msgs` (text and binary messages only, on decrypted connections) is the
  **model requests written** number the participant, status, heartbeat (`model_requests`) and server
  see. **It is a request count, not a call count.** It includes warm-ups, interrupts and other POSTs
  to listed hosts (run 1 had 27 warm-ups, 37 interrupts and `/models` calls). Real calls, models and
  tokens come from the usage reader (PRD-09, `coverage.wire_calls`). Nobody calls this number "calls".
- **When a request counts** (ledger R6: a model call counts only once its bytes are in the record).
  `http_req` counts `http_req` OPENs written. `http_post` counts `http_req` streams on decrypted
  connections with method `POST` whose END is written (a request counts once its body is in the
  record; `req_holes` says which have holes). `ws_up_msgs` counts `ws_up` DATA with the end-of-message
  flag and a text or binary opcode, when that DATA is written.
- `hosts`: model requests written per decrypted host, at the same counting point. `tunnel_hosts`: per
  tunnelled host, connections and bytes each way; a listed host's value adds `listed` (the GAP reason
  name of its tunnels; null for an unlisted host). PRD-08 recounts both.
- `req_whole`: those requests whose stream (or message) ended `complete` with no gap. `req_holes`: the
  rest that ended. The heartbeat, status and end screen show `req_whole` next to `model_requests`
  whenever `req_holes` is above 0, so a request whose body is in a gap is never shown as whole.
- `gaps`, `lost_bytes`, `lost_unknown` and `off_ms` cover the GAPs before the COUNT in its boot, like
  every counter. `lost_bytes` sums known `lost - 1`; `lost_unknown` counts GAPs with `lost` 0; `off_ms`
  is the length of `recorder_down` gaps. A recovery frame's `recorder_down` GAP comes after that
  frame's COUNT, so it is added in the entry total below.
- `open_streams` and `held_bytes` are gauges at the moment of the COUNT; `held_peak` is the boot's peak.
- **Entry total.** Counts are per boot. The entry total is, over boots, each boot's last COUNT (for a
  dead boot, the COUNT in its recovery frame, which recounts that boot up to the last kept LINK):
  counters summed, maps summed per key, `held_peak` the largest, gauges left out. Then, for every
  `recorder_down` GAP in a recovery frame, add 1 to `gaps` and `lost_unknown` and its length to
  `off_ms`. The seal's `totals` carry this sum, also for a `recovered` seal; PRD-08 recounts with the
  same rule.
- **Counts that leave the machine** (the heartbeat's `model_requests` and `counts`, PRD-06 and PRD-07)
  are the counters as they stood at the durable point's LINK, never ahead of it, so the server never
  shows a count that a crash could take back.

## 10. Links: the hash chain stays, one link per flush

**Decision: keep a hash chain, cut it to one link per flush point.**

```
hash(n)   = SHA-256( "GCR1-LINK" || hash(n-1) || covered bytes )
hash(-1)  = 32 zero bytes
covered   = every raw byte from the end of LINK n-1 (or the start of the entry) up to and
            including LINK n's own kind, len, seq, wall_ns and mono_ns; everything but its hash
```

- The writer encodes the LINK without its hash, hashes, appends the hash, then flushes the compressor,
  then writes the index line.
- **When a LINK is written** (section 6 and the table at the top cite this): 1 s after the last LINK if
  records were written since, sooner when 16 MiB raw wait, before rotation, at the seal, and after each
  COUNT (so at least once a minute while the recorder lives).
- A PART header is covered by the next link like any record. Because a link covers its own times, the
  wall time of every record is under the chain too.
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

## 11. The seal and events

The last records of a sealed entry are:

```
GAP cut_at_seal ... / END cut_at_seal ...          what was still open
GAP after_stop                                     only when bytes were relayed after the stop
EVENT {"ev":"count", ...}                          final counts of this boot
EVENT {"ev":"seal","reason":"harness_exit","exit":{"code":0},"harness":"exited",
       "workspace_head":"<40 hex or null>","dirty":"<sha256 hex of git status --porcelain -z>",
       "boots":1,"totals":{ ...COUNT keys summed over boots, section 9... }}
LINK
(frame end, idx line E)
```

- `reason` is one of `harness_exit`, `deadline` (the recorder's own deadline timer), `server_stop`
  (the heartbeat answered `stop`; `server_reason` names the server's reason, for example `deadline` or
  `replaced`), `seal_command`, `signal`, or `recovered` (a dead entry closed by `continue` or `seal`
  after both processes died; PRD-02 section 9.4).
- `exit` is `{"code":n}`, `{"signal":n}` or null. `harness` is `exited`, `running` (the seal came while
  it ran) or `unknown` (`recovered`).
- A `recovered` seal sits in the recovery frame: the dead boot's COUNT (from `counts.jsonl`, section
  12 step 3), `GAP recorder_down`, the seal with `totals` (section 9's entry total, `tunnel_hosts`
  included), LINK, frame end, `R` line.
- Nothing follows the seal's LINK. There is no seal call: the server seals the entry when the final
  heartbeat names the seal LINK's `seq` and hash and the server holds the chain up to it (PRD-07). The
  Worker refuses anything past it (`server/worker/src/chunks.ts:60-62`).
- `seal.json`, `baseline.json`, `receipt.json`, `acks.jsonl` and the tunnel list file are gone. The seal,
  the start facts and the tunnels are records.

Events, all JSON with `ev`:

| `ev` | When | Keys |
|---|---|---|
| `start` | First record after PART, once per boot | `run`, `entry`, `segment` (the entry's segment number from the server's run approval: 0 for `start`, then 1, 2 for each `continue`; PRD-07's `segment`), `goalcup` (version), `build` (40-hex commit the binary was built from, or null), `os`, `arch`, `connector`, `connector_source` (`builtin` or `file`), `connector_sha256`, `harness_version`, `hosts`, `hosts_sha256` (SHA-256 of the host list file as read: the embedded list or a `--hosts` file), `mask_rules`, `mask_values`, `fd_limit`, `keep` (true when `GOALCUP_KEEP_PARTS=1`), `deadline`, `server_time`, `local_wall_at_server_time`, `workspace_head`, `resume` (true when boot > 0) |
| `prev_boot` | Boot > 0 | How the previous boot ended, from the watchdog: `{"code":n}` or `{"signal":n}` |
| `launch` | Harness started | `pid`, `program` (file name only) |
| `exit` | Harness ended | `code` or `signal` |
| `count` | Every 60 s, and before the seal | Section 9 |
| `clock` | Wall time jumped | `jump_ns` |
| `suspend` | The machine slept | `ns` |
| `note` | Anything a person should see later | `what`, and plain facts, never secret values. `what` is one of: `long_tunnel`, `ca_refused`, `informational_response`, `connect_refused`, `parse_stopped`, `fd_limit`, `writer_slow`, `outlived`, `native_unreadable`, `native_symlink`, `native_missing`, `native_other_device`, `snapshot_unstable`, `final_snapshot_live`, `local_model`, `mask_values`. Readers show an unknown `what` as is |
| `seal` | Last | Above |

Facts of the notes other PRDs read: `long_tunnel`: `host`, `why` (`stream` or `upload`), `conns`, `up`,
`down`, `secs`. `parse_stopped`: `conn`, `reason`. `connect_refused`: `host` (only when the head parsed),
`status`. `outlived`: `count`, `programs`. `native_missing`: `row`. `native_other_device`: `row`, `path`
(relative to the source's root). An unsupported body coding is named by GAP 3's `codings` tail, not by a
note.

`hosts` in the `start` event is the list in force, by source, with each entry's kind and
certificate-pinning flag (`pinned`):
`{"shared":[{"host":"api.openai.com","kind":"provider","pinned":false},{"host":"*.openai.azure.com",...}],
"participant":[{"host":"llm.example.com","kind":"other","pinned":false}],"config":[...]}`. Participant and
config entries have kind `other`. A reader takes kinds from here or from `conn` meta `kind`, never from
a host list built into the reader, so a `--hosts` run is read with its own kinds.

`local_model` (review A finding 7) is written when a hint or a harness config names a loopback or
private address: calls to a model on this computer bypass the proxy (`NO_PROXY`) and are not recorded.
PRD-11 writes it; PRD-10 words it.

## 12. Compression, chunks and crash recovery

**Compression.** The writer compresses as it writes: zstd level 3, `window_log` 27, long distance
matching on, content checksum off, content size off, no dictionary. Each part file is one frame (plus
its recovery frames, section 12). The compressor is flushed at every LINK, so every index line is a
block boundary.

- `PartWriter::open(dir, PartOptions)`. `PartOptions { window_log: 27, rotate_at: 512 MiB,
  rotate_after: 60 min }` are the defaults; tests use `window_log` 20 so they do not hold 141 MiB each
  (`golden.rs` and `recovery.rs` keep 27). `window_log` is a parameter for tests only: the recorder
  always writes window 2^27 with long matching (ledger F2, locked). PRD-02 sets `rotate_at` under its
  test hook (`GOALCUP_TEST_ROTATE_MB`); `goalcup-format` itself has no test hooks.
- Memory: about 141 MiB to compress with window 2^27 and long distance matching (measured with
  `zstandard` 0.25 on 300 MiB of synthetic data, appendix M section 4.5). 128 MiB of window to
  decompress.
- Why 27: `zstd -d` accepts windows up to 2^27 with no flag (appendix M section 4.4; libzstd's default
  decoder limit). A larger window would need `--memory` on every reader and about 600 MiB on the
  participant's machine.
- Why write compressed: the part file is the upload. No spool, no second compressor, and the window
  covers the whole part file instead of one chunk. Run 1 at `-3` without a long window was 3.2 to 1
  because the previous body of the same agent fell out of a 2 MiB window when 20 agents interleaved.
- Flushing once a second costs a few bytes per block; the window history is kept across flushes.

**Chunks** (PRD-06 sends them, PRD-07 stores them):

- A chunk is the compressed bytes of one part file from one index line (or the file start) to a later
  index line. It never spans two part files. It is at most 32 MiB (the Worker's cap,
  `server/worker/src/chunks.ts:22`). A flush interval is at most 16 MiB raw, so one always fits.
- Headers keep their names and meaning: `Goalcup-Length` = raw bytes covered (always above zero);
  `Goalcup-First-Seq` = first link seq in it; `Goalcup-Last-Seq` and `Goalcup-Head` = the end line's
  link; `Goalcup-Prev` = the link hash at its start; `Goalcup-Sha256` = SHA-256 of the body; the path
  offset is the raw offset of its first byte. Hashes in headers are 64 lowercase hex characters;
  `Goalcup-Prev` of an entry's first chunk is 64 zeros (`hash(-1)`, the database default `head`).
- A chunk is **not** decodable on its own: it continues its part file's frame. That is the price of the
  long window across a whole part file. Order and integrity are enforced by offsets, links and the
  SHA-256 in the R2 key.

**What the Worker needs: nothing new for the format.** It stores bytes and never decompresses
(`server/worker/src/chunks.ts:199-227`). `Content-Type: application/zstd` and the key stay. Signing of
acks goes (PRD-07).

**What fetch needs.** `scripts/fetch-run.sh` lists an entry's chunks by offset, checks each SHA-256,
and concatenates them in offset order into one file. It does not decode. The verifier and the usage
reader decode with window 2^27 (PRD-08, PRD-09); `zstd -dc --long=27` is the manual way. Today's
per-chunk decode (`scripts/fetch-run.sh:327`) goes (PRD-07). An entry still in progress ends inside an
open frame: the decoder outputs everything up to the last chunk and reports the open end, which is
expected.

**Crash recovery** (the recorder died; the watchdog starts the next boot, or `close_dead_entry` closes
the entry):

1. If recovery runs under a new `boot_id`, first drop index lines past the durable index length in
   `cursors.json` (section 1). A part file with no index line left (its boot died before its first
   LINK there) is deleted with its index. Then read the last remaining part file's index. Drop index
   lines whose `c_end` is past the file's length. Truncate the file to the last remaining line's
   `c_end`. Bytes after it were never uploaded (the uploader sends only up to the durable point).
2. If that line is `L`, the frame is open: append `01 00 00` (if it is `E` or `R`, the frame is
   already closed; skip this step). Those 3 bytes are a zstd block header
   with Last_Block 1, type raw, size 0 (RFC 8878 section 3.1.1.2): an empty last block. Because every
   flush ends on a block boundary and the frame declares no content size and no checksum, this closes
   the frame validly whatever the encoder wrote before it. Checked on 30 Oct with the system libzstd
   1.5.5 through its C API: a frame opened with level 3, window 2^27 and long matching (frame header
   `28 b5 2f fd 00 88`: no content size, no checksum, window 2^27), flushed twice, closed with
   `01 00 00` and followed by a new frame, decodes as one stream with the default decoder limits. The
   builder repeats this in Rust first (`tests/zstd_close.rs`, builder notes).
3. Append one small recovery frame holding the dead boot's COUNT (its counters at the kept last LINK:
   the `counts.jsonl` line whose `seq` equals that LINK's, PRD-02 section 9.2), then `GAP` (`sid` 0,
   `recorder_down`, start = the last LINK's wall and record clock time, end = now, `lost` 0 = unknown,
   tail `{"cause": ...}`), then a `LINK`. Write an `R` line. This part file is now closed, and its next
   chunk has raw bytes, so the Worker accepts it. A part file already closed by `R` (the boot after a
   recovery died before its first index line, and step 1 deleted its part file) takes another recovery
   frame holding only the GAP and a LINK: its boot's COUNT is already in the first.
4. Start a new part file: PART with `boot` = the last remaining PART's boot plus one (so a deleted
   boot's number is reused and boot numbers have no hole), `prev` = the recovery LINK's hash, then
   `start` with `resume: true`, then `prev_boot`. (For a dead entry, step 3's frame also holds the
   `recovered` seal after the GAP, and nothing follows.)

If no part file with an index line remains (the entry never wrote one; pruning always keeps the last
such file, section 1), delete what is there and start boot 0 at raw offset 0 with a `recorder_down` gap
right after its `start` event: start = the dead boot's start time if the watchdog passed it, else the
deleted part file's modification time; end = now; `lost` 0; tail `{"cause": ..., "exits": n}` (how many
boots died this way).

**Fallback if step 2 fails in Rust** (written now so nobody improvises): recovery does not touch the old
part file beyond step 1. It starts a new part file whose first records after PART and `start` are the
`recorder_down` GAP and a LINK. `read.rs` then decodes part files one by one and accepts a part file that
ends inside a frame exactly at an index line. The cost is that `cat rec-*.gcr.zst | zstd -dc` stops at
the first crashed part file, so fetch and the verifier must decode per part file. Use it only if
`zstd_close.rs` fails.

## 13. Invariants (PRD-08 checks every one, `inv1` to `inv14`)

1. The stream starts with PART (part 0, boot 0) and every part file starts with PART. Boot numbers have
   no hole (a boot that left no index line is deleted and its number reused, section 12).
2. Links: `seq` runs 0, 1, 2 without a hole across part files and boots; every hash recomputes.
3. Every DATA, END, GAP (non-zero `sid`) and MASK follows the OPEN of its `sid` in the same boot.
4. Per stream, DATA offsets are contiguous from `meta.from` (0 when absent), except where a GAP of that
   `sid` starts at the hole and its known lost bytes fill it exactly. A GAP with unknown loss (`lost` 0)
   must sit at the hole; the hole's size is then reported, not checked. `conn` streams have no DATA, and
   their GAPs (`off` 0) are exempt.
5. Every MASK lies inside bytes already written for its `sid`, and those bytes are all `*`.
6. Every boot after 0 is preceded by `recorder_down` gaps in the recovery frames just before its PART:
   one per boot that died since the last kept LINK, together running from that LINK to the recovery
   with no hole. Boot 0 may instead have one `recorder_down` gap just after its `start` event, when
   earlier attempts died before any index line. A `recovered` seal is preceded by exactly one
   `recorder_down` gap in its own recovery frame.
7. Every COUNT equals a recount of the records before it in its boot (a record belongs to the boot of
   the nearest PART before it, so a recovery frame's COUNT recounts the dead boot up to the last kept
   LINK). The seal's `totals` equal section 9's entry total.
8. A sealed entry ends with `seal`, then LINK, then a frame end. Nothing follows.
9. Every `http_resp` names a `req` that is an `http_req` on the same `conn`, or 0.
10. **Streams close.** In a sealed entry every stream has an END, or belongs to a boot that is followed by
    a `recorder_down` gap. Streams still open at the seal end `cut_at_seal`.
11. **`seen` adds up.** For every END of a non-`conn` stream, `seen` equals the end of the last written
    DATA or known GAP of that stream (`off + len`, or `off + lost - 1`), and is never below `meta.from`.
    A stream with an unknown-loss GAP is reported, not checked.
12. **A GAP has an extent.** `end_mono > start_mono`, or `lost > 1`. `end_wall` is not before
    `start_wall`. When the GAP's tail names another `boot_id` (section 6), wall times are used:
    `end_wall > start_wall`, or `lost > 1`.
13. **COUNT cadence.** Within each boot, two COUNT events (or the PART and the first COUNT) are at most
    180 s apart on the record clock, after taking off the `ns` of `suspend` events between them, until
    the boot's last LINK.
14. **Native resume has no hole.** Within an entry, for one native `path` and `gen`, a later stream's
    `from` is at or below the end of the earlier stream's written bytes. An overlap after a crash is
    expected (bytes past the last durable point are read again); a hole is not, unless a GAP covers it.

## 14. Size: run 1 under the new format

Payload stays 721.8 MB (the masked bytes are the same length). Base64 (239.3 MB) and per-record
chain framing (343.9 MB) go. What replaces them:

| Piece | Records (est.) | Bytes each (est.) | MB (est.) |
|---|---:|---:|---:|
| WebSocket server messages, one DATA each | 152,728 | 12 | 1.8 |
| WebSocket client messages, DATA pieces of at most 256 KiB | about 1,300 | 12 | under 0.1 |
| HTTP request and response bodies, DATA pieces | about 3,000 | 12 | under 0.1 |
| HTTP OPENs with masked headers in JSON | 1,460 | about 1,000 | 1.5 |
| Connection and WebSocket direction OPENs and ENDs | about 1,000 | about 200 | 0.2 |
| Session file DATA (43 files polled once a second while growing) | about 30,000 | 14 | 0.4 |
| LINKs (at most one a second for 2 hours) | at most 7,200 | 52 | 0.4 |
| COUNT and other events | about 150 | 600 | 0.1 |
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
- **200 agents at once** (estimate, 10 times run 1's wire rate, ratio 3.2 to 1): about 8 GB raw and
  2.5 GB compressed an hour, about 6 Mbit/s of upload, about 400 GB a week. A home uplink under that
  keeps the server hours behind, and part files wait on disk compressed. There is no cap (ledger R5);
  this is the cost to plan for.

## 15. What the usage reader can rely on (PRD-09)

1. Every byte the proxy saw on a decrypted connection is in a DATA record, or inside a GAP with its
   offset, or counted as unknown loss in a GAP, or inside a `conn` GAP when the connection stopped
   parsing. Nothing is dropped without a record.
2. HTTP bodies are decoded (no coding left; `coding` lists what was undone), de-chunked, and masked the
   same length. JSON bodies parse unless a gap cut them; SSE responses are plain text.
3. Every response names its request (`req`), every request names its connection (`conn`), and
   every connection names its host, whether it was on a list (`listed`) and the host's `kind`.
4. WebSocket messages keep their boundaries (end-of-message flag) and their direction (`ws_up`,
   `ws_down`), and are inflated. A message's time is its first piece's time. For a message of several
   pieces, the last piece's time is the closest thing to an end time; it is not exact.
5. Session files: `path` and `gen` identify a file; DATA offsets are file offsets; a stream may start
   at `from`, and after a crash the next boot's stream may repeat bytes already written: dedupe by
   (`path`, `gen`, offset). A new generation follows a `source_changed` end. Database copies are
   grouped by `set`; `harness_exited` says whether the harness still ran.
6. Every record has a time; every LINK maps it to wall time; `suspend` events mark sleep; the server's
   `received_at` bounds clock skew.
7. Every COUNT is an exact recount of its boot up to that COUNT. Seal totals are section 9's entry
   total; PRD-08 recounts with the same rule (`counts_match_seal`). They are not call counts.
8. The reader must not rely on: a model name, a thread id or a token count being in any fixed place;
   message boundaries in HTTP bodies; masked bytes meaning anything; any order across streams other
   than time.
9. Decoding needs the whole part file from its start (128 MiB window, so not inside a Worker's 128 MB
   isolate). Run the reader as an organizer job or on the pad (PRD-09).

## 16. Worked example

One HTTP/1.1 POST with a streamed reply, then a link. Entry `e0123456789abcdef`, boot 0, part 0, wall
time 2026-11-07T00:00:00Z (1,794,009,600,000,000,000 ns), record clock 5,000,000,000 ns, `boot_id`
16 bytes of `0x11`. Every byte below is fixed by the rules above and the JSON bodies given here; the
bytes and hashes were computed by a script on 30 Oct.

JSON bodies, exactly (sorted keys, compact). The `start` event is shortened for the example; a real one
has every key of section 11.

```
start   {"build":"0123456789abcdef0123456789abcdef01234567","connector":"codex-cli","connector_source":"builtin","entry":"e0123456789abcdef","ev":"start","goalcup":"1.0.0","mask_rules":["openai_key","bearer"],"resume":false,"run":"r0123456789abcdef"}
conn    {"alpn":"http/1.1","fingerprint":"0123456789abcdef","host":"api.openai.com","listed":"shared","mode":"decrypt","port":443,"reason":null,"tls":"1.3"}
req     {"coding":null,"headers":[["authorization","********************"],["content-type","application/json"]],"masked":["authorization"],"method":"POST","path":"/v1/responses","ver":"1.1"}
resp    {"coding":["br"],"decoded":true,"headers":[["content-type","text/event-stream"],["content-encoding","br"]],"masked":[],"status":200,"ver":"1.1"}
END 3   {"wire":41}
```

(The example's `conn` meta predates the `kind` key; the golden keeps it as is, which also proves that
readers ignore a missing optional key.)

| Raw offset | Bytes | Record | First bytes (hex) |
|---:|---:|---|---|
| 0 | 90 | PART | `01 58 47 43 52 31 01 65 30 31 32 ...` (kind 1, len 88, `GCR1`, ver 1, `e012...`) |
| 90 | 246 | EVENT `start` | `07 f3 01 00 7b 22 62 75 ...` (kind 7, len 243, dt 0, JSON) |
| 336 | 158 | OPEN `conn` sid 1 | `02 9b 01 01 80 89 7a 01 00 00 7b ...` (sid 1, dt +1 ms, kind 1, conn 0, req 0, JSON) |
| 494 | 193 | OPEN `http_req` sid 2 | `02 be 01 02 80 92 f4 01 02 01 00 7b ...` (sid 2, dt +2 ms, kind 2, conn 1, req 0) |
| 687 | 35 | DATA sid 2 | `03 21 02 80 89 7a 00 00 7b 22 6d 6f ...` (dt +1 ms, off 0, flags 0, 27 body bytes) |
| 722 | 6 | END sid 2 | `04 04 02 00 01 1b` (dt 0, complete, seen 27) |
| 728 | 156 | OPEN `http_resp` sid 3 | `02 99 01 03 80 8c 8d 9e 02 03 01 02 7b ...` (dt +300 ms, kind 3, conn 1, req 2) |
| 884 | 46 | DATA sid 3 | `03 2c 03 80 ad e2 04 00 00 64 61 74 61 ...` (dt +5 ms, off 0, 37 bytes) |
| 930 | 19 | END sid 3 | `04 11 03 80 89 7a 01 25 7b 22 77 ...` (dt +1 ms, complete, seen 37, tail) |
| 949 | 49 | LINK seq 0 | `08 2f 00 80 f3 f8 b0 9f c7 e6 f2 18 80 d7 80 e4 13` then `hash` |

The LINK's hash is SHA-256 of the 9 ASCII bytes `GCR1-LINK`, 32 zero bytes and raw bytes 0 to 965
(everything before the LINK, plus the LINK's first 17 bytes: kind, len, seq, wall, time):
`87e927badc48e694b3f55d0e8f129502bb8b27c605c25e05bd8bc8616dde8dbd`. The whole 998-byte raw stream has
SHA-256 `013c82339391fbb8af8b6c8d296fa7ad841fdbcf62266a10bd35bc16d3ee4898`. Framing is 934 of 998 bytes
here because the example has 64 bytes of payload; on real traffic the same records carry kilobytes to
megabytes. Full bytes:

```
0000  0158474352310165303132333435363738396162636465660000008080909d9e
0020  c7e6f21880e497d0121111111111111111111111111111111100000000000000
0040  000000000000000000000000000000000000000000000000000007f301007b22
0060  6275696c64223a22303132333435363738396162636465663031323334353637
0080  38396162636465663031323334353637222c22636f6e6e6563746f72223a2263
00a0  6f6465782d636c69222c22636f6e6e6563746f725f736f75726365223a226275
00c0  696c74696e222c22656e747279223a2265303132333435363738396162636465
00e0  66222c226576223a227374617274222c22676f616c637570223a22312e302e30
0100  222c226d61736b5f72756c6573223a5b226f70656e61695f6b6579222c226265
0120  61726572225d2c22726573756d65223a66616c73652c2272756e223a22723031
0140  3233343536373839616263646566227d029b010180897a0100007b22616c706e
0160  223a22687474702f312e31222c2266696e6765727072696e74223a2230313233
0180  343536373839616263646566222c22686f7374223a226170692e6f70656e6169
01a0  2e636f6d222c226c6973746564223a22736861726564222c226d6f6465223a22
01c0  64656372797074222c22706f7274223a3434332c22726561736f6e223a6e756c
01e0  6c2c22746c73223a22312e33227d02be01028092f4010201007b22636f64696e
0200  67223a6e756c6c2c2268656164657273223a5b5b22617574686f72697a617469
0220  6f6e222c222a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a225d2c5b22636f
0240  6e74656e742d74797065222c226170706c69636174696f6e2f6a736f6e225d5d
0260  2c226d61736b6564223a5b22617574686f72697a6174696f6e225d2c226d6574
0280  686f64223a22504f5354222c2270617468223a222f76312f726573706f6e7365
02a0  73222c22766572223a22312e31227d03210280897a00007b226d6f64656c223a
02c0  226d31222c22696e707574223a226869227d04040200011b02990103808c8d9e
02e0  020301027b22636f64696e67223a5b226272225d2c226465636f646564223a74
0300  7275652c2268656164657273223a5b5b22636f6e74656e742d74797065222c22
0320  746578742f6576656e742d73747265616d225d2c5b22636f6e74656e742d656e
0340  636f64696e67222c226272225d5d2c226d61736b6564223a5b5d2c2273746174
0360  7573223a3230302c22766572223a22312e31227d032c0380ade2040000646174
0380  613a207b2274797065223a22726573706f6e73652e636f6d706c65746564227d
03a0  0a0a04110380897a01257b2277697265223a34317d082f0080f3f8b09fc7e6f2
03c0  1880d780e41387e927badc48e694b3f55d0e8f129502bb8b27c605c25e05bd8b
03e0  c8616dde8dbd
```

`goalcup-verify dump` prints (the golden compares parsed JSON values, not bytes):

```json
{"o":0,"k":"part","entry":"e0123456789abcdef","boot":0,"part":0,"raw_offset":0,"wall":"2026-11-07T00:00:00Z","mono":5000000000,"next_link":0}
{"o":90,"k":"event","t":5000000000,"ev":"start","run":"r0123456789abcdef","entry":"e0123456789abcdef","goalcup":"1.0.0","build":"0123456789abcdef0123456789abcdef01234567","connector":"codex-cli","connector_source":"builtin","mask_rules":["openai_key","bearer"],"resume":false}
{"o":336,"k":"open","sid":1,"t":5001000000,"skind":"conn","meta":{"host":"api.openai.com","port":443,"mode":"decrypt","listed":"shared","reason":null,"alpn":"http/1.1","tls":"1.3","fingerprint":"0123456789abcdef"}}
{"o":494,"k":"open","sid":2,"t":5003000000,"skind":"http_req","conn":1,"meta":{"method":"POST","path":"/v1/responses","ver":"1.1","headers":[["authorization","********************"],["content-type","application/json"]],"masked":["authorization"],"coding":null}}
{"o":687,"k":"data","sid":2,"t":5004000000,"off":0,"len":27,"text":"{\"model\":\"m1\",\"input\":\"hi\"}"}
{"o":722,"k":"end","sid":2,"t":5004000000,"reason":"complete","seen":27}
{"o":728,"k":"open","sid":3,"t":5304000000,"skind":"http_resp","conn":1,"req":2,"meta":{"status":200,"ver":"1.1","headers":[["content-type","text/event-stream"],["content-encoding","br"]],"masked":[],"coding":["br"],"decoded":true}}
{"o":884,"k":"data","sid":3,"t":5309000000,"off":0,"len":37,"text":"data: {\"type\":\"response.completed\"}\n\n"}
{"o":930,"k":"end","sid":3,"t":5310000000,"reason":"complete","seen":37,"tail":{"wire":41}}
{"o":949,"k":"link","seq":0,"wall":"2026-11-07T00:00:00.310Z","mono":5310000000,"hash":"87e927badc48e694b3f55d0e8f129502bb8b27c605c25e05bd8bc8616dde8dbd"}
```

The same shape for a WebSocket, a loss, a mask and a pinned tunnel (dump lines only, abridged):

```json
{"k":"open","sid":7,"skind":"ws_up","conn":4,"req":6,"meta":{"deflate":true}}
{"k":"open","sid":8,"skind":"ws_down","conn":4,"req":6,"meta":{"deflate":true}}
{"k":"data","sid":7,"off":0,"flags":{"fin":true,"op":"text"},"len":1500000,"note":"6 pieces, the last with fin"}
{"k":"data","sid":8,"off":0,"flags":{"fin":true,"op":"text"},"len":412}
{"k":"gap","sid":7,"reason":"memory_guard","start":"...:00.100Z","end":"...:02.900Z","off":1500000,"lost":1480001,"tail":{"guard":"held_bytes","limit":536870912}}
{"k":"mask","sid":8,"off":120,"len":51,"rule":"openai_key"}
{"k":"open","sid":9,"skind":"conn","meta":{"host":"api.anthropic.com","mode":"tunnel","listed":"shared","kind":"provider","reason":"pinned"}}
{"k":"gap","sid":9,"reason":"pinned","off":0,"lost":88213,"start":"...:03.000Z","end":"...:41.200Z"}
{"k":"end","sid":9,"reason":"complete","seen":0,"tail":{"up":4096,"down":84116}}
```

## Files it owns

Create:

- `crates/goalcup-format/Cargo.toml`. Dependencies, at today's locked versions (`Cargo.lock`): `sha2`
  0.10.9, `serde_json` 1.0.151, `zstd` 0.13.3 (with `zstd-safe` 7.3.0 and `zstd-sys` 2.1.1, libzstd
  1.5.7). Nothing async, no TLS.
- `crates/goalcup-format/src/lib.rs` (constants, kinds, reasons, enums, guard and note names)
- `crates/goalcup-format/src/codec.rs` (varint, zigzag, encode and decode of every record)
- `crates/goalcup-format/src/link.rs` (link hash, chain check)
- `crates/goalcup-format/src/part.rs` (`PartWriter`, `PartOptions`: compressor settings, flush at LINK,
  index lines, rotation order, fsync points, recovery steps 1 to 4)
- `crates/goalcup-format/src/read.rs` (streaming reader over part files or a fetched concatenation;
  tolerant open end; index reader `read_index()`)
- `crates/goalcup-format/src/dump.rs` (one JSON line per record, used by PRD-08)
- `crates/goalcup-format/tests/zstd_close.rs`, `tests/golden.rs`, `tests/roundtrip.rs`,
  `tests/recovery.rs`
- `crates/goalcup-format/tests/data/worked-example.gcr` (the 998 raw bytes of section 16) and
  `tests/data/worked-example.dump.jsonl` (section 16's dump)
- `docs/RECORD-FORMAT.md` is written by PRD-15 from this revision.

Must not touch: anything under `crates/goalcup/` (PRD-02 and the module owners), `server/`
(PRD-07), `crates/goalcup-verify/` (PRD-08, PRD-09), the root `Cargo.toml` and `Cargo.lock` (the
integrator adds `crates/goalcup-format` to `members` inside this PRD's first merge, PRD-16 section 2
step 5).

## Interfaces with other PRDs

| PRD | Uses | Seam |
|---|---|---|
| PRD-02 | `PartWriter::{open(dir, PartOptions), recover, write(&Record), link(), rotate_if_due(), seal()}`, `Record`, `GapReason`, `EndReason`, `StreamKind`, durable point with the counters at its LINK; `counts.jsonl` line per LINK for the recovery frame's COUNT; GAP reasons 1, 2, 10, 12, 15; seal reasons incl. `recovered`, `server_stop`; notes `fd_limit`, `writer_slow`, `outlived`; the `suspend` event; the entry-total rule (section 9) | The writer thread owns one `PartWriter` |
| PRD-03 | `StreamKind`, meta keys (`kind`, `fingerprint`, `tls` null, pseudo-headers, `coding` list), END tails (`up`, `down`, `plain_up`, `plain_down`, `wire`, `trailers`, `h2_error`, `close_code`, `unsent`), GAP reasons 1 (incl. `hello_too_big` tunnels), 3 (tail `codings`), 4, 6, 7, 11 (stream and `conn` GAP only, never a tunnel reason), 13, 14 and the `conn` GAP rule, guard names, notes `long_tunnel`, `ca_refused`, `informational_response`, `connect_refused`, `parse_stopped`; `path` without query | Through PRD-02's recorder handle, never directly |
| PRD-04 | `native` meta `from`, `fid`; `snapshot` meta `fid`, `set`, `harness_exited`; END `source_changed` without GAP; GAP 5 tail `unread_at_least`; GAP 10 `cut_at_seal`; notes `native_unreadable`, `native_symlink`, `snapshot_unstable`, `final_snapshot_live`, `native_missing`, `native_other_device`. "Last END `seen` equals the file size" holds as stated: `seen` is an end offset, not a sum over boots | Through PRD-02 |
| PRD-05 | The MASK record, the `*` fill rule, `rule` as a kind index, `fail_closed`, `mask_values` counts, no `"` in a span | PRD-05 returns spans; PRD-02 writes MASK records |
| PRD-06 | `IndexLine`, `read_index()`, write order and rotation order (section 1), chunk header rules (section 12), counts at the durable point | Reads `.idx` and part file bytes, never decodes |
| PRD-07 | Chunk headers, key, fetch rule (concatenate, check, never decode), seal by final heartbeat | `scripts/fetch-run.sh` |
| PRD-08 | `read`, `dump`, invariants 1 to 14; `seen` and `plain_up`/`plain_down` definitions; reserved reasons 8 and 9 shown as `other(n)` | Every invariant in section 13 is a check |
| PRD-09 | Section 15; host kinds from `conn` meta or the `start` event, never embedded | Reads fetched record streams |
| PRD-10 | COUNT keys incl. `hosts`, `tunnel_hosts`; GAP reasons (final numbers above); seal reasons and `harness`; notes `local_model`, `mask_values` | Participant text names reasons in plain words |
| PRD-11 | `start` keys `connector_source`, `hosts` with `kind` and the certificate-pinning flag (`pinned`), `hosts_sha256`; note `local_model` | Through PRD-02 |
| PRD-12, PRD-16 | `start` keys `build`, `keep`, `hosts_sha256` and the certificate-pinning flag (`pinned`) | Read from the record and the server copy |
| PRD-14 | `PartOptions { window_log, rotate_at }`, the worked-example files | Tests |
| PRD-17 | Lessons that touch the format (deflate, brotli, h2, `unsent`, `coding` list) | Lesson cases H10 and K15 of PRD-17 write a part file through the real pipeline and read it with `read`; their test also runs `goalcup-verify check` and asserts exit 0. No other lessons test writes a part file |

## What is cut

The v5 envelope, chain and capture contracts: `crates/goalcup-protocol/src/{envelope.rs, chain.rs,
capture.rs, capture_status.rs, export.rs, ack.rs, receipt.rs, seal.rs, provenance.rs, run_secret.rs}`,
4,351 lines by `wc -l` (3,949 src, 402 inline tests, PRD-13's count), and their tests, deleted with the
old crates by PRD-13. `run_secret.rs` is copied in part first (about 330 lines into PRD-05's
`src/mask/values.rs`). No reader for v5 is kept (no backward compatibility). The v5 corpus fixtures go
with them. The run 1 converter (a v5 to GCR1 rewrite of run 1's server copy) is not built (review B
N1): it needs a v5 reader, read access to run 1's copy and its own tests, and serves no point that a
real pad run and PRD-08's synthetic `check_run1_shape.rs` do not.

## Acceptance

Real-run proofs first (run on the pad, PRD-12, with `GOALCUP_KEEP_PARTS=1`):

1. **First real run decodes.** A Claude Code direct-key pad run, `--prompt smoke` (PRD-12: one subagent,
   about 3 minutes), kept parts: `cat rec-*.gcr.zst |
   zstd -dc` exits 0; `goalcup-verify check` passes every invariant in section 13; COUNT recounts
   match. Pass: zero invariant failures.
2. **Server copy equals local.** The same run's fetched chunks, concatenated, equal the concatenated
   local part files byte for byte (parts kept, so nothing was pruned). Pass: identical SHA-256.
3. **Kill test.** During a Codex pad run (`full` prompt), `kill -9` the recorder twice, 2 minutes
   apart, while a response streams (no test hooks). Pass: the harness finishes its turn with no
   participant action; the entry decodes; exactly two `recorder_down` gaps, start before end, each
   under 2 s long; links verify across three boots; the server copy equals local.
4. **Size.** On the first Codex pad run with subagents, report raw bytes, compressed bytes and framing
   share. Pass: framing (all bytes that are not DATA payload) under 2% of raw. Compare the ratio with
   section 14 in the report; no pass line on the ratio.

Automated tests that guard them:

- `zstd_close.rs` (first): with the locked `zstd` crate, an encoder at level 3, window 27, long
  matching, no checksum, no content size, flushed twice, then `01 00 00` appended and a second frame
  written, decodes as one stream; the frame header has no content size and no checksum flag.
- `golden.rs`: the records of section 16 encode to `tests/data/worked-example.gcr` (998 bytes, SHA-256
  `013c8233...`) and decode back; the link hash is `87e927ba...`; `dump` output parses to the same JSON
  values as `worked-example.dump.jsonl`.
- `roundtrip.rs`: random records of every kind round trip; unknown kinds are skipped; a body over
  16 MiB is refused; a truncated record at the end is reported, not read; a MASK between two timed
  records leaves the second record's time unchanged (MASK carries no time).
- `recovery.rs`: a part file cut at a random byte recovers to the last index line, closes with
  `01 00 00` and a recovery frame, and the concatenated part files decode as one stream with one
  `recorder_down` gap; every part file decodes alone too. A last part file with no index line is
  deleted and its boot number reused; a part file already closed by `R` takes a second recovery frame;
  the recovery frame's COUNT equals a recount of its boot.

## Risks and blind spots

- **Binary is opaque to people.** Mitigated by `dump` and the documented Python reader. A participant
  who wants to read their own record needs one of them.
- **A chunk is not decodable alone.** One lost chunk makes the rest of its part file unreadable. The
  Worker stores create-only with SHA-256 in the key, and run 1's server copy held every byte; still, a
  server bug now costs up to one part file (512 MiB raw), not one chunk.
- **The 141 MiB compressor** sits in the participant's process for the whole run. The window stays
  2^27 (ledger F2, locked); a smaller window needs the owner.
- **Masking with `*` inside a SQLite WAL frame breaks that frame's checksum.** Decided (PRD-04): record
  as is; the reader repairs checksums before opening. A mask inside a page header or b-tree byte cannot
  be repaired; that copy is unreadable and the reader says so (PRD-09).
- **Length-preserving masks show a secret's length.** A planner default (PRD-05), listed in PRD-00's
  defaults for the owner.
- **Write-time compression** puts zstd on the writer thread. If it falls behind, PRD-02's memory guard
  writes gaps; it never slows the relay. Peak throughput with long distance matching is unmeasured on
  the target machines.
- **Window reach** with many large-context agents at once (section 14).
- **`kern.bootsessionuuid`**, `mach_absolute_time` for the suspend event, and the zstd crate's setter
  names for window and long matching are unverified until built.
- **One link a second** is about 31 MB a busy week of hashes that do not compress.
- **More keys.** Revision 2 adds about 30 meta, tail, event and count keys. None needs parsing on the
  hot path: each is a value the proxy, reader or writer already holds. `tunnel_hosts` in every COUNT
  grows with the number of tunnelled hosts (tens, rarely hundreds); it compresses well.
- **`after_stop` and `local_model`** only help if PRD-02 and PRD-11 write them. If they do not, the
  numbers stay unused; nothing else breaks.

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Rotation | 512 MiB raw or 60 minutes | PRD-06 after measuring |
| Link and flush | at most 1 s, or 16 MiB raw | PRD-02 |
| fsync | every 5 s, at rotation and at the seal | PRD-02 |
| DATA piece | at most 256 KiB | PRD-02 |
| COUNT | every 60 s; verifier bound 180 s | PRD-02 |
| zstd level | 3 | PRD-06, with a measured reason (level 1 first) |
| zstd window 2^27 with long matching | locked (ledger F2) | the owner only |
| Clock jump note, suspend event | over 2 s | PRD-02 |

## Builder notes

Order of work:

1. `tests/zstd_close.rs` against the locked crate, before `part.rs`. If it fails, take the fallback of
   section 12 and say so in the night log.
2. `lib.rs` and `codec.rs`: kinds, reasons, names, varint and zigzag, encode and decode, bounds. Commit
   `tests/data/worked-example.gcr` from section 16's hex and make `golden.rs` pass before anything else.
   Other builders start from these types, so publish them within the first hour, even with `todo!()`
   in `part.rs`.
3. `link.rs`, then `part.rs` (writer, index, rotation order, fsync), then recovery.
4. `read.rs` over a directory of part files and over a fetched concatenation; `dump.rs`.

Traps:

- The zstd crate's `Encoder` flushes a block on `Write::flush` and writes the end block on `finish`.
  Set no content checksum and no content size: either would make the hand-closed frame in recovery
  invalid. Check that the window and long-matching setters exist on the encoder in the locked zstd
  version (or use its raw parameter setter) before relying on them.
- Never write an `L` line for the last link of a closed part file (section 1). Otherwise the uploader
  can send the 3 end bytes alone, with a raw length of 0, which the Worker and the shared database
  refuse (`length > 0`, `server/worker/sql/goal-cup-tracer-upload.sql:206`).
- The LINK hash covers the LINK's own fields; encode them, hash, then append the hash.
- `dt` is signed. Do not sort records by time when writing.
- `seen` is an end offset (`from` plus bytes seen), not a byte count, for native streams resumed at a
  cursor. For wire streams the two are equal.
- Masked meta values keep their length; do not reuse today's `<redacted:kind>` marker.
- `lost` is bytes plus one. Writing the byte count itself turns "1 byte lost" into "none lost".

How a watcher proves it in a real run: after any pad run, run `goalcup-verify check <entry dir>` and
`goalcup-verify check --server <fetched dir>`, then compare the two SHA-256 sums of the concatenated
part files. Kill the recorder once with `kill -9` in one pad run per lane and check acceptance 3.

## Review answers

History from revision 2. Where a row below differs from the body (for example GAP 11 as a tunnel
reason, or the `unsupported_coding` note), the body and the "Alignment answers" section win.

Sources: CC = `prd-consistency.md` item; A = review A finding; B = review B item; "PRD-NN ask n" = the
ask of PRD-01 in that PRD. Verdicts: accepted, partly accepted, rejected.

| Item | Ask | Verdict and what changed |
|---|---|---|
| CC 28, B C2, PRD-02 ask 1, PRD-03 ask 2, PRD-05 ask 7, PRD-08 ask 8, PRD-10 A3, PRD-15 ask 4 | GAP reason 12 claimed twice | Accepted. 12 `mask_failed`, 13 `upstream_untrusted`. Section 3 table is final |
| CC 28, PRD-02 ask 2, PRD-04 ask 7 | Reasons 8 and 9 never written | Accepted. Marked reserved, never written by 1.0; PRD-08 shows them as `other(n)` |
| CC 87 | `write_error` claim in PRD-13 and PRD-15 | Accepted here (reason 8 reserved; the cause sits in `recorder_down`). PRD-13 and PRD-15 reword their lines |
| CC 29, PRD-02 ask 3, PRD-04 ask 2, PRD-08 ask 1 | Streams start at `meta.from` | Accepted. `from` on `native`; invariant 4 counts from it; new invariant 14 forbids a hole across boots |
| CC 30, PRD-04 ask 2, PRD-09 ask 1, CC 62 | `fid`, `harness_exited`, `set` | Accepted. Section 4. `set` is `"<boot>.<n>"` |
| CC 31, PRD-02 ask 4, PRD-10 A2 | Seal reasons `recovered`, server `stop` | Accepted. Added `recovered` and `server_stop` (every server `stop`, its reason in `server_reason`, as PRD-00 revision 2 states). Seal gains `harness` |
| CC 32, PRD-03 asks 1 and 3 | GAP on a `conn` stream; tunnel reasons `sign_in`, `alpn_other` | Accepted. `conn` GAP rule in section 4; invariant 4 exempts `conn`. Added GAP 14 `protocol_other` so `alpn_other` and `not_tls` on a listed host have a named loss. PRD-00's registry lists 12 and 13 only; 14 and 15 are PRD-01's own numbers and PRD-00 should copy them. PRD-08's `listed_hosts_decrypted` accepts every named listed-host tunnel reason (6, 7, 11, 13, 14), as CC 52 asks |
| CC 33, PRD-03 ask 4, PRD-08 ask 6, PRD-17 ask 1 | END tails | Accepted. `up`/`down` are TCP wire bytes; `plain_up`/`plain_down` for `decrypt`; `wire`, `trailers`, `h2_error`, `close_code`, `unsent` |
| CC 34, PRD-17 ask 2 | `coding` as a list | Accepted |
| CC 35, PRD-03 asks 5 and 6 | `tls` null, `fingerprint`, HTTP/2 pseudo-headers | Accepted |
| CC 36, PRD-02 ask 5, PRD-04 ask 5 | Note values; `pinned_host` | Accepted the six. `pinned_host` dropped: PRD-03 never writes it and GAP `pinned` already names each connection; PRD-10 keys its line off the GAP |
| CC 37, PRD-02 ask 5, PRD-05 ask 6, PRD-11 asks 1 and 2, PRD-12 asks 1 and 2, PRD-16 asks 1 and 2 | `start` keys | Accepted all: `fd_limit`, `mask_values` (counts per source plus `skipped`), `connector_source`, host `kind` and `pinned`, `hosts_sha256`, `keep`, `build` |
| CC 110 | Host kinds and `other` | Accepted, plus `kind` on `conn` meta so PRD-09 never re-implements host matching |
| CC 38, PRD-10 A1 | `queue_*` against `held_*`; `hosts`, `tunnel_hosts` | Accepted. Renamed to `held_bytes`, `held_peak`. `hosts` and `tunnel_hosts` in COUNT and so in seal totals (a recovered seal sums COUNTs). PRD-16 X3 reads `held_peak` |
| CC 39, PRD-08 asks 2 to 5 | Invariants | Accepted as invariants 10 to 13 (closure, `seen`, extent, COUNT cadence). Invariant 13 subtracts sleep, which needed the new `suspend` event: without it every laptop that sleeps would fail |
| CC 40, PRD-02 ask 6, CC 56 | Local files never uploaded; name the folder | Accepted. Table in section 1 names each file, its folder and its owner |
| CC 40, PRD-06 B1 to B3 | Rotation fsync order, drop encoder first, index after bytes | Accepted. Section 1 "Write order" and "Rotation" |
| CC 40, CC 94, PRD-14 ask 2 | `window_log` and `rotate_at` as parameters | Accepted. `PartOptions`; the hook stays in PRD-02 |
| CC 40, PRD-14 ask 1, B A1 | Golden bytes cannot be rebuilt from the text | Accepted. Section 16 now gives every JSON body, the full 998-byte hex, the link hash and the file hash, computed on 30 Oct. The old hash (`3149d18a...`) is void: the start event and meta changed and the link now covers its own fields. The dump golden compares parsed values, not bytes |
| CC 40, PRD-15 ask 1 | "Part" means two things | Accepted. "Part file" in prose; "part" is the participant's entry |
| CC 40, PRD-04 ask 1 | Path may hold workspace text | Accepted. Section 5 |
| CC 40, PRD-05 asks 4 and 5 | `rule` indexes kinds; spans over several DATA; no `"`; `meta.masked` | Accepted. Section 8 |
| CC 40, PRD-07 asks 8 to 10 | Seal by heartbeat, fetch does not decode, counts at the durable point | Accepted. Sections 11, 12, 9 |
| CC 40, PRD-09 ask 3 | "Model requests written" is not a call count | Accepted. Section 9 |
| CC 40, PRD-08 ask 7, PRD-09 ask 4, PRD-14 ask 3, B N1 | Run 1 converter | Rejected as a PRD-01 deliverable (B N1 wins): cut from acceptance and files. PRD-08 and PRD-09 use `check_run1_shape.rs` and real pad runs; if the owner wants the converter later it is a by-hand tool outside the gate |
| CC 58 | Guard names | Accepted. Listed under the GAP table |
| CC 89, PRD-13 ask 7 | v5 size; `run_secret.rs` | Accepted. 4,351 lines (3,949 src); copied in part by PRD-05 |
| PRD-04 asks 3, 4, 6 | `source_changed` END with no GAP; `unread_at_least`; `-shm` and WAL repair | Accepted. Sections 3 and 4; risk row closed |
| PRD-09 ask 2 | WebSocket message end time | Accepted. Section 15 item 4 |
| PRD-15 asks 2 and 3 | A settled revision; Python reader on stdin | Accepted. This is revision 2; the reader reads decompressed bytes on standard input (section 2) |
| B C3 | About 30 missing keys | Accepted (rows above). None adds hot-path parsing |
| B A2 | Hand close depends on the Rust crate | Accepted. RFC basis and a libzstd check written in section 12; `zstd_close.rs` is step 1; a fallback is written |
| B W10, PRD-06 A5 | Server copy equals local needs kept parts | Accepted. Acceptance runs with `GOALCUP_KEEP_PARTS=1` |
| B T2 | Proofs needing test hooks | No change needed: acceptance 3 is a plain `kill -9` |
| A 2 | Parser loss on a reused connection is silent | Partly accepted: the format part. A `conn` GAP from the fault to the close counts the unparsed bytes. PRD-03 writes it; PRD-08 decides whether `conn_bytes` fails; the ALPN preference is PRD-03's |
| A 3 | Fail-closed masking can star a whole direction | Partly accepted: the format part. `fail_closed` rule kind and `mask_failclosed_bytes` in COUNT. The message-boundary rule is PRD-05's |
| A 7 | Local model calls never named | Accepted the format part: note `local_model`. PRD-11 writes it, PRD-10 words it |
| A 8 | Calls after the seal are lost unnamed | Partly accepted: GAP 15 `after_stop` counts them in the seal. Keeping the listener open, the subreaper and port reuse are PRD-02's |
| A 9 | SQLite subagents only at the seal | Partly accepted: the format already allows a database copy at any time (`form: "final"`, `harness_exited`, `set`). Whether to copy hourly is PRD-04's |
| A 10 | Decoder memory outside the guard | Partly accepted: guard name `decoder_window` reserved. The rule is PRD-03's |
| A 14 | Timers name no clock | Accepted the format part: section 6 says every timer uses the record clock |
| A 12 | Values added mid-run | Partly accepted: a `note` `mask_values` with counts if PRD-05 adds values; the watch itself is PRD-05's |
| A 1, 4, 5, 6, 11, 13, 15, 16 | Proxy, watchdog, masking and upload findings | No format change needed; owned by PRD-02, PRD-03, PRD-05, PRD-06, PRD-11 |
| Own check | Link did not cover its own times | Fixed: the link hash now covers the LINK's `seq`, `wall_ns` and `mono_ns`, so every wall time is under the chain |
| Own check | `seen` across resumed native streams | Fixed: `seen` is an end offset; PRD-04's "last END `seen` equals size" holds without summing |

## Alignment answers

Revision 3, from the alignment fix lists (g1, g2, g3), checked against OWNER-DECISIONS.md. Where two fix
lists disagreed, the ledger decided.

| Id | What was done |
|---|---|
| L2-04 | Applied with G2-21: a dead boot's last COUNT is the COUNT in its recovery frame, taken from `counts.jsonl` at the kept last LINK (sections 9, 11, 12 step 3, invariant 7) |
| L4-2 | Applied with G2-20: risk row and defaults now say window 2^27 is locked (ledger F2); only the level is PRD-06's |
| L4-5 | Applied: `path` and HTTP/2 `:path` drop the query, as in 0.7; section 8 masks header values and URL userinfo only (ledger M1, point 4). PRD-05 and PRD-03 make their part |
| L2-10 | Applied with P04-5: note `native_other_device` (`row`, `path`) |
| L2-16 | Applied with P03-4's names: tunnel reason `hello_too_big`, close GAP 1 `memory_guard` with `{"guard":"client_hello","limit":65536}` on a listed host. A GAP reason is not reused as a `conn` reason, so `memory_guard` is not a tunnel reason |
| L2-17 | Applied: `http_post` counts at the written END of a POST `http_req` (ledger R6: a call counts once its bytes are in the record). PRD-02 section 6 must move from OPEN to END |
| G2-20 | Applied: defaults split into level (PRD-06) and window with long matching (owner only); `window_log` is a test-only parameter (section 12) |
| G2-05 | Applied: root `Cargo.toml` and `Cargo.lock` added to "Must not touch" |
| G2-09 | Applied: off span starts at the last LINK named by an index line (section 7); acceptance 3 rewritten |
| G2-21 | Applied: section 9 entry total (last COUNT per boot plus recovery-frame gaps), record-to-boot rule, recovery frame order COUNT, GAP, (seal), LINK for every recovery, section 15 item 7, invariant 7 |
| G2-26 | Applied: `lost` is in the stream's offset space; unknown size gives `lost` 0 and tail `wire` |
| G2-27 | Applied with G3-X20: notes `outlived`, `native_missing`, `parse_stopped` and their facts; `tunnel_hosts` gains `listed`; GAP 10 written by PRD-02 and PRD-04; PRD-04 interface row |
| P01-2 | Applied, adjusted: a part file with no index line is deleted and its boot number reused, so invariant 1 keeps "boot 0" and adds "no hole". A closed (`E` or `R`) last part file takes a recovery frame without step 2. The no-line case uses the deleted file's time when the watchdog gave none, so the GAP always has an extent. Pruning now keeps the last part file with an index line, so recovery always has one when the entry wrote any (PRD-06 should copy that clause). Invariant 6 allows one gap per dead boot |
| P01-5 | Applied: GAP tail `boot_id` and wall times for invariant 12 and off time |
| P01-7 | Applied: MASK carries no time and does not move the running time; `roundtrip.rs` checks it. The golden bytes are unchanged |
| P03-4 | Applied (see L2-16). PRD-08 counts it as `named` |
| P03-5 | Applied: `h2_unsupported` removed from tunnel reasons and from the listed-tunnel GAP list; GAP 11 stays for streams and the `conn` GAP of a connection that stops parsing (ledger H6) |
| P04-5 | Applied (see L2-10) |
| G2-33 | Applied: section 9 says `req_whole` is shown beside `model_requests` when `req_holes` is above 0 |
| P01-11 | Applied |
| P01-12 | Applied |
| P01-13 | Applied: durable index length in `cursors.json`; recovery under a new `boot_id` drops lines past it (sections 1, 12) |
| P01-14 | Applied |
| P01-15 | Applied: "certificate-pinning flag (`pinned`)" |
| P01-16 | Applied |
| P01-17 | Partly: `ws_up_msgs` counts at the written end-of-message DATA, as asked. `http_post` counts at the written END, not the OPEN, because ledger R6 (locked) counts a call only once its bytes are in the record (L2-17 wins) |
| P01-19 | Applied (section 8 and the risk row). PRD-00 adds the default to its list |
| P01-20 | Applied |
| P01-9 | Applied: one cadence sentence in section 10; section 6 and the table point to it |
| P03-15 | Applied: note `connect_refused` (`host`, `status`) |
| P03-9 | Applied: GAP 3 tail `codings`; the `unsupported_coding` note is dropped from the note list and the PRD-03 row, since the GAP names the codings |
| P05-11 | Applied |
| X1-7 | Applied: "200 agents at once" bullet in section 14 |
| X2-11 | Partly: this pass edits only this file, so the move to `REVIEW-HISTORY.md` is left to the step that owns that file. Every ruling still binding is in the body, and a line above the review table says the body wins |
| G3-X20 | Applied (see G2-27) |
| G3-X33 | Applied: PRD-17 interface row |

## Amendments

### 2026-10-30, replan after PRD-08 slice 1 (merge d10feef7)

- **Ask from PRD-08 (judge 13:06Z), optional.** `goalcup-verify` counts S6 `bytes.raw` and finds a
  torn tail by wrapping the part files in its own `Raw` reader and `Meter` (`input.rs`, about 40
  lines) beside `read::Reader`. If `read::Reader` or its `Tail` reported the raw bytes decoded and
  whether input ended inside an open frame, PRD-08 would use `read::decode` and drop the copy. A
  format-core slice may add it; nothing waits on it. It is a `goalcup-format` edit, so it is PRD-01's
  to make, with a seam line naming the two new fields.
- `goalcup-verify dump` uses `goalcup_format::dump::Dumper` unchanged; the worked example's dump
  through it equals the committed file as JSON (judge 12:35Z box).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, conductor seam line from PRD-02 core step 2 (08:50Z)

Two shapes PRD-02 writes, accepted here: GAP reason 12 (mask_failed) may carry lost 1 with no stream byte lost and tail {"meta": "open"|"tail"|"event"} when metadata masking panicked; DATA may be empty (0 bytes) only with fin, for an empty WebSocket message. The codec already accepts both. PRD-08 recounts them as PRD-02's Counter does.
