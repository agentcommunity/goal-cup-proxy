# The record format (GCR1)

This page is for contributors and anyone who wants to read a record by hand. The design and its
reasons are in [PRD-01](plans/2026-10-30-v1.0/PRD-01-record-format.md) and
[D76](DECISIONS.md#d76-the-record-format-is-gcr1-and-the-part-file-is-the-upload-unit). Where this
page differs from PRD-01 or the code, they are right: `crates/goalcup-format/src` for framing,
records and links, and the recorder (PRD-02, PRD-05) for counts, gaps, masks and the seal.

One entry (one `goalcup start` or `goalcup continue`) is one record stream. It holds every
recorded byte raw, every loss as a gap with a start and an end, and counts of what was written.

## Files on disk

One entry has one folder, `$GOALCUP_HOME/runs/<run_id>/<entry_id>/rec/`:

```
rec-00000000000000000000.gcr.zst   part file 0, starts at raw offset 0
rec-00000000000000000000.idx       its flush index
rec-00000000000536870982.gcr.zst   part file 1, starts at raw offset 536,870,982
rec-00000000000536870982.idx
```

- **Part files.** A part file is named after its first raw offset, 20 digits, zero padded. Join the
  decompressed bytes of every part file in name order and you get the entry's record stream. A raw
  offset is a position in that stream. Only part files are uploaded.
- **Index files** (`.idx`) are local and never uploaded. One text line per flush point:
  `L <seq> <raw_end> <c_end> <hash>` (a flush inside an open frame), `E ...` (the frame and the
  part file are closed) or `R ...` (a recovery frame was added and the part file is closed). `seq`
  and `hash` are the LINK's, `raw_end` is the raw offset just past that LINK, and `c_end` is the
  part file's length at that point. A line is written only after the bytes up to `c_end`. A last
  line without a newline is ignored.
- **Rotation.** A part file is closed when it holds 512 MiB of raw records, or when it has been open
  60 minutes on the record clock and holds a link. Closing writes a LINK, ends the frame and writes
  one `E` line. There is never an `L` line for the last link of a closed part file.
- **Local files.** `cursors.json` (the durable index length, for recovery after a reboot) and
  `counts.jsonl` (one COUNT per LINK, for the recovery frame) are the recorder's, not the format's.

## Framing

Every record is `kind` (one byte), `len` (a varint) and then `len` bytes of body.

- A **varint** is unsigned LEB128: 7 bits per byte, low bits first, the top bit set on every byte
  but the last, at most 10 bytes. A **zigzag** number maps signed to unsigned (0, -1, 1, -2 become
  0, 1, 2, 3) and is then written as a varint.
- Kind `0x00` is never valid. A reader **skips an unknown kind by its length**. That is how a later
  1.x adds kinds. An unknown record still counts for the link hash.
- A reader refuses a `len` over 16 MiB (`MAX_BODY`), so a corrupt file cannot make it allocate.
- JSON bodies are compact UTF-8. Key order means nothing, and readers ignore keys they do not know.
- An optional JSON tail (END, GAP) is the rest of the body. No tail means no bytes, not `{}`.

## Records

`sid` is a stream id, `dt` a zigzag time step (see Time). All times are nanoseconds.

| Kind | Name | Body, in order |
|---|---|---|
| 0x01 | PART | `GCR1` (4 bytes), version 1 (u8), entry id (17 ASCII bytes), `boot`, `part`, `raw_offset`, `wall_ns`, `mono_ns` (varints), `boot_id` (16 bytes), `next_link` (varint), `prev` (32 bytes) |
| 0x02 | OPEN | `sid`, `dt`, `skind` (u8), `conn`, `req` (varints, 0 when none), `meta` JSON (rest) |
| 0x03 | DATA | `sid`, `dt`, `off` (varint), `flags` (u8), the bytes (rest) |
| 0x04 | END | `sid`, `dt`, `reason` (u8), `seen` (varint), optional JSON tail |
| 0x05 | GAP | `sid` (0 means all streams), `reason` (u8), `start_wall`, `end_wall`, `start_mono`, `end_mono`, `off`, `lost` (varints), optional JSON tail |
| 0x06 | MASK | `sid`, `off`, `len`, `rule` (varints) |
| 0x07 | EVENT | `dt`, JSON with key `ev` (rest) |
| 0x08 | LINK | `seq`, `wall_ns`, `mono_ns` (varints), `hash` (32 bytes) |

- PART is the first record of every part file. `prev` is the hash of the last LINK before it
  (32 zero bytes for the first part file) and `next_link` the `seq` the next LINK will have.
- DATA holds masked bytes at stream offset `off`. `flags` bit 0 (`FLAG_FIN`) marks the end of a
  WebSocket message; bits 4 to 7 hold its opcode (1 text, 2 binary, 8 close, 9 ping, 10 pong).
  Both are 0 outside WebSocket streams. Bits 1 to 3 are 0.
- END `reason`: 1 `complete`, 2 `interrupted`, 3 `decode_error`, 4 `cut_at_seal`, 5
  `source_changed`. `seen` is the stream offset just past the last source byte seen, written or
  lost. For a `conn` stream it is 0; its tail holds bytes each way (`up`, `down`).

## Stream kinds

`skind` in OPEN. A stream is an OPEN, its DATA records and an END.

| n | Kind | One stream is |
|---|---|---|
| 1 | `conn` | One client connection through the proxy, decrypted or tunnelled. No DATA. `meta` has `host`, `port`, `mode` (`decrypt` or `tunnel`), `listed`, `kind`, `reason` |
| 2 | `http_req` | One request body. `meta` holds the head: `method`, `path` (no query), `ver`, `h2` (HTTP/2 stream id, absent for 1.1), masked `headers`, `masked`, `coding` |
| 3 | `http_resp` | One final response body. `req` names its request. Bodies are de-chunked and decoded. `meta` has `status`, `ver`, `h2`, `headers`, `masked`, `coding`, `decoded` |
| 4 | `ws_up` | Every message the client sent on one WebSocket, inflated, in order. `meta.deflate` is true when permessage-deflate was in use |
| 5 | `ws_down` | Every message the server sent on it. `meta` as `ws_up` |
| 6 | `native` | One generation of one append-only session file. `meta.path` is `<source>:<relative path>` |
| 7 | `snapshot` | One whole-file copy at one moment |

A number this build does not know is shown as `other(n)`.

## IDs

- Run `r` plus 16 hex and entry `e` plus 16 hex come from the server.
- `boot` counts recorder processes inside one entry, from 0, with no hole.
- `sid` starts at 1 and is never reused inside a boot. The global key is `<entry>.<boot>.<sid>`.
- A connection is its `conn` stream's `sid`; an exchange is its `http_req` stream's `sid`.
- A session file's ID is its `meta.path` plus `meta.gen`, the same across boots and entries.
- LINK `seq` runs from 0 per entry, across part files and boots.

## Time

- The record clock is monotonic and includes sleep. `boot_id` names it. Times from different
  `boot_id` values are never compared; use wall time across them.
- PART, LINK and GAP carry full times (a GAP's time is its `end_mono`). OPEN, DATA, END and EVENT
  carry `dt`, a **signed** step from the previous timed record in file order. The first record after
  a PART steps from the PART's `mono_ns`.
- MASK carries no time and does not move the running time. Neither does an unknown kind.
- Every LINK pairs `wall_ns` with `mono_ns`. Map a record to wall time with the nearest LINK before
  it. `clock` and `suspend` events note wall time jumps and sleep.
- **Never sort by time.** Pieces from many streams meet in one queue, so times can step backwards in
  file order. File order is the truth.

## Gaps and off spans

Every loss is one GAP with a start, an end, a reason and the lost bytes when known. `lost` is
**bytes lost plus one**: 0 means unknown, 1 means none. `off` is the stream offset where the loss
starts (0 for `sid` 0 and for `conn` streams).

| n | Reason | What happened |
|---|---|---|
| 1 | `memory_guard` | A stated memory guard was passed; tail `{"guard", "limit"}` |
| 2 | `recorder_down` | The recorder was not running, or ran in null mode. Always `sid` 0 |
| 3 | `unsupported_encoding` | A body coding the proxy cannot decode; tail `codings` |
| 4 | `decode_failed` | A coding or the framing broke part way |
| 5 | `source_changed` | A session file changed with bytes still unread |
| 6 | `tls_refused` | A listed host's client refused the run CA; tunnelled |
| 7 | `pinned` | A listed host's client pins certificates; tunnelled |
| 8 | reserved | Never written. A write error ends the recorder instead. Shown as `other(8)` |
| 9 | reserved | Never written. A failed read is retried. Shown as `other(9)` |
| 10 | `cut_at_seal` | Bytes still unread or unwritten when the seal's time ran out |
| 11 | `h2_unsupported` | An HTTP/2 feature the proxy cannot follow; the rest relays unparsed |
| 12 | `mask_failed` | The masker failed on a piece, so the piece was dropped |
| 13 | `upstream_untrusted` | A listed host's server certificate failed the check; tunnelled |
| 14 | `protocol_other` | A listed host spoken to in a protocol the proxy does not parse; tunnelled |
| 15 | `after_stop` | Bytes relayed after the seal began. `sid` 0 |

**Off spans.** Recording is off only while the recorder is not running or runs in null mode. That
time is a `recorder_down` GAP from the last kept LINK to the next boot (or to the recovery of a dead
entry). The sum of those gaps is the off time. A GAP whose end was taken under another `boot_id`
than its start carries tail key `boot_id`; its length, for off time and invariant 12, is `end_wall`
minus `start_wall`, never the two record clock times.

## Masks

A masked byte is replaced in place by `*` (0x2A), so the length stays the same. Offsets stay equal
to source offsets and JSON stays parseable. Each masked span gets a MASK record (`sid`, `off`,
`len`, `rule`), written after the DATA it covers, one per DATA record it touches. `rule` indexes the
`start` event's `mask_rules` list, which names kinds of secret, never values. MASK has no time.
Masked header values in `meta` keep their length too, and `meta.masked` names those headers.

## Written counts

A `count` EVENT holds counts of records **already written before it** in the same boot, so a
reader can recount it exactly. A record belongs to the boot of the nearest PART before it, so a
recovery frame belongs to the dead boot. `http_post` plus `ws_up_msgs` is the number of **model
requests written**. It is a request count, not a call count: warm-ups and other POSTs to listed
hosts count too.

- `http_post` counts a POST `http_req` stream on a decrypted connection once its END is written,
  not at its OPEN.
- `ws_up_msgs` counts `ws_up` DATA with the end-of-message flag and a text or binary opcode (so not
  ping, pong or close), when that DATA is written.
- The **entry total** takes each boot's last COUNT (for a dead boot, the COUNT in its recovery
  frame), sums counters and maps per key, keeps the largest `held_peak` and leaves gauges out. Then,
  for every `recorder_down` GAP in a recovery frame, it adds 1 to `gaps` and `lost_unknown` and the
  gap's length to `off_ms`. The seal's `totals` are this entry total.

## Links

One LINK per flush chains everything before it:

```
hash(n)  = SHA-256( "GCR1-LINK" || hash(n-1) || covered bytes )
hash(-1) = 32 zero bytes
covered  = every raw byte after LINK n-1 (or from the start of the entry) up to and including
           LINK n's kind, len, seq, wall_ns and mono_ns: everything but its own hash
```

To recompute: start SHA-256 with the 9 bytes `GCR1-LINK` and the previous hash, feed every raw byte
after the previous LINK, then the LINK's bytes except its last 32, and compare with those 32. The
recorder chooses when to link; the writer flushes the compressor at every LINK and then writes the
index line.

## The seal and events

EVENT JSON always has `ev`: `start` (once per boot, with `mask_rules`), `prev_boot`, `launch`,
`exit`, `count`, `clock`, `suspend`, `note` (with `what`) and `seal`. A sealed entry ends with what
was still open (`cut_at_seal` gaps and ends), an `after_stop` GAP if needed, the last COUNT, the
`seal` event, a LINK and the frame end. A `recovered` seal (a dead entry closed later) sits in a
recovery frame instead: the dead boot's COUNT, a `recorder_down` GAP, the `seal` event, a LINK, the
frame end and an `R` line. Nothing follows the seal's LINK.

## Compression, chunks and crash recovery

- Each part file is one zstd frame (plus recovery frames): level 3 (`LEVEL`), window 2^27
  (`WINDOW_LOG` 27, 128 MiB), long distance matching, no checksum, no content size. Part files
  joined together are one valid zstd stream: `cat rec-*.gcr.zst | zstd -dc --long=27`.
- A chunk the uploader sends is the compressed bytes of one part file from one index line, or from
  the file start, to a later index line. It usually spans many flushes and never two part files. It
  is not decodable alone; decode a whole part file from its start.
- A live entry's last part file ends inside an open frame. `goalcup-verify dump` (the Rust decoder)
  gives every record up to the last flush and reports the open end. That is expected. The `zstd`
  command line tool may stop short of the last flush on such a file, or on an entry not yet
  recovered after a crash, so counts made through it are then a lower bound. For a live entry, use
  `goalcup-verify dump`.
- **Recovery** after a crash: drop index lines past the part file's length (and, after a reboot,
  past `cursors.json`); delete a part file with no index line left; cut the last part file at its
  last line's `c_end`. If that line is `L`, close the frame by hand with the 3 bytes `01 00 00`
  (`ZSTD_END_BLOCK`, an empty last raw block). Then add a recovery frame: the dead boot's COUNT
  (unless an earlier recovery frame holds it), a `recorder_down` GAP, a LINK and an `R` line. The
  next boot starts a new part file. An entry already sealed is left as it is.

## Invariants

1. The stream starts with PART (boot 0, part 0); every part file starts with PART; boots have no
   hole.
2. LINK `seq` runs 0, 1, 2 with no hole across part files and boots, and every hash recomputes.
3. Every DATA, END, GAP (non-zero `sid`) and MASK follows its stream's OPEN in the same boot.
4. DATA offsets are contiguous from `meta.from` (or 0), except where a GAP of that stream fills the
   hole.
5. Every MASK lies inside bytes already written for its stream, and those bytes are all `*`.
6. Every boot after 0 follows `recorder_down` gaps from the last kept LINK to recovery (boot 0: at
   most one, after `start`).
7. Every COUNT equals a recount of its boot before it; the seal's `totals` equal the entry total.
8. A sealed entry ends with `seal`, then LINK, then a frame end.
9. Every `http_resp` names a `req` on the same `conn`, or 0.
10. In a sealed entry every stream has an END, or its boot is followed by a `recorder_down` gap.
11. A non-`conn` END's `seen` equals the end of the stream's last DATA or known GAP. A stream with
    an unknown-loss GAP is reported, not checked.
12. A GAP has an extent: `end_mono` after `start_mono`, or `lost` above 1.
13. In each boot, COUNT events are at most 180 s apart on the record clock, less sleep.
14. A session file resumed in a later stream starts at or below what was written before.

## The worked example

`crates/goalcup-format/tests/data/worked-example.gcr` is one HTTP/1.1 POST with a streamed reply
and a link: 998 raw bytes, 10 records. `crates/goalcup-format/tests/golden.rs` checks it. Its dump
(`worked-example.dump.jsonl`, made by `crates/goalcup-format/src/dump.rs`) is below. Its `conn`
meta has no `kind`: the example predates that key and is kept as is, which also shows that readers
ignore a missing optional key.

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

The LINK hash is SHA-256 of `GCR1-LINK`, 32 zero bytes and raw bytes 0 to 965:
`87e927badc48e694b3f55d0e8f129502bb8b27c605c25e05bd8bc8616dde8dbd`. The whole file has SHA-256
`013c82339391fbb8af8b6c8d296fa7ad841fdbcf62266a10bd35bc16d3ee4898`.

## A Python reader

This reader needs only the Python standard library (3.9 or later). It reads the decompressed stream
from standard input and prints each record kind with its count and a total. On a live entry its
counts are a lower bound (see the `zstd` note above):

```
cat rec-*.gcr.zst | zstd -dc --long=27 | python3 read.py
```

```python
import sys

KINDS = {1: "part", 2: "open", 3: "data", 4: "end", 5: "gap", 6: "mask", 7: "event", 8: "link"}
MAX_BODY = 16 << 20


def length(src, offset):
    """The varint after a kind byte: (value, its size), or None if the input ends first."""
    value = 0
    for i in range(10):
        b = src.read(1)
        if not b:
            return None
        if i == 9 and b[0] > 1:
            sys.exit(f"len at raw offset {offset} is over 64 bits or 10 bytes")
        value |= (b[0] & 0x7F) << (7 * i)
        if b[0] < 0x80:
            return value, i + 1


def main():
    src = sys.stdin.buffer
    kinds, offset = {}, 0
    while True:
        head = src.read(1)
        if not head:
            break
        kind = head[0]
        if kind == 0:
            sys.exit(f"kind 0 at raw offset {offset}: not a GCR1 stream")
        found = length(src, offset)
        if found is None:
            print(f"cut record at raw offset {offset}")
            break
        if found[0] > MAX_BODY:
            sys.exit(f"record at raw offset {offset} is over 16 MiB")
        body = src.read(found[0])
        if len(body) < found[0]:
            print(f"cut record at raw offset {offset}")
            break
        if offset == 0 and (kind != 1 or body[:4] != b"GCR1"):
            sys.exit("the stream does not start with a GCR1 PART record")
        name = KINDS.get(kind, f"unknown({kind})")  # an unknown kind is skipped by its length
        kinds[name] = kinds.get(name, 0) + 1
        offset += 1 + found[1] + len(body)
    for name, n in kinds.items():
        print(f"{name} {n}")
    print(f"total {sum(kinds.values())} records, {offset} raw bytes")


main()
```

On the worked example (`python3 read.py < worked-example.gcr`, already raw) it prints `part 1`,
`event 1`, `open 3`, `data 2`, `end 2`, `link 1` and `total 10 records, 998 raw bytes`.
